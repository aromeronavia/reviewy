use std::fmt;
use std::process::Command;

use anyhow::{Context, Result, anyhow, bail};

/// A GitHub repository identified by its owner and name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoSlug {
    pub owner: String,
    pub name: String,
}

impl RepoSlug {
    /// Parses `owner/name`.
    pub fn parse(spec: &str) -> Result<RepoSlug> {
        let parts: Vec<&str> = spec.split('/').collect();
        match parts.as_slice() {
            [owner, name] if !owner.is_empty() && !name.is_empty() => Ok(RepoSlug {
                owner: owner.to_string(),
                name: name.to_string(),
            }),
            _ => bail!("expected a repository in the form owner/name, got `{spec}`"),
        }
    }

    /// Parses the GitHub repository out of a git remote URL, in any of the
    /// forms git uses: `git@github.com:o/n.git`, `https://github.com/o/n`,
    /// `ssh://git@github.com/o/n.git`, with or without `user:password@`.
    pub fn from_remote_url(url: &str) -> Result<RepoSlug> {
        let url = url.trim();
        let path = github_path(url).ok_or_else(|| {
            anyhow!("remote `{}` is not a GitHub remote", redact_userinfo(url))
        })?;
        let path = path
            .trim_end_matches('/')
            .trim_end_matches(".git")
            .trim_end_matches('/');
        RepoSlug::parse(path)
    }

    /// Reads the `origin` remote of the git repository in the current directory.
    pub fn detect_from_git() -> Result<RepoSlug> {
        let output = Command::new("git")
            .args(["remote", "get-url", "origin"])
            .output()
            .context("failed to run git")?;
        if !output.status.success() {
            bail!("not a git repository or no origin remote; pass owner/name explicitly");
        }
        let url = String::from_utf8_lossy(&output.stdout);
        RepoSlug::from_remote_url(&url)
    }
}

/// The part of a remote URL after the GitHub host, or `None` when the
/// remote points somewhere else.
fn github_path(url: &str) -> Option<&str> {
    if let Some(path) = url.strip_prefix("git@github.com:") {
        return Some(path);
    }
    let (_, after_scheme) = url.split_once("://")?;
    let host_and_path = after_scheme
        .rsplit_once('@')
        .map_or(after_scheme, |(_, rest)| rest);
    host_and_path.strip_prefix("github.com/")
}

/// Replaces any `user:password@` in a URL so it can be echoed safely.
fn redact_userinfo(url: &str) -> String {
    match (url.split_once("://"), url.rsplit_once('@')) {
        (Some((scheme, _)), Some((_, host_and_path))) => {
            format!("{scheme}://<redacted>@{host_and_path}")
        }
        _ => url.to_string(),
    }
}

impl fmt::Display for RepoSlug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_owner_slash_name() {
        let r = RepoSlug::parse("ratatui/ratatui").unwrap();
        assert_eq!((r.owner.as_str(), r.name.as_str()), ("ratatui", "ratatui"));
    }

    #[test]
    fn rejects_missing_slash() {
        assert!(RepoSlug::parse("ratatui").is_err());
    }

    #[test]
    fn rejects_extra_segments() {
        assert!(RepoSlug::parse("a/b/c").is_err());
    }

    #[test]
    fn parses_ssh_remote() {
        assert_eq!(
            RepoSlug::from_remote_url("git@github.com:owner/name.git").unwrap().to_string(),
            "owner/name"
        );
    }

    #[test]
    fn parses_https_remote_with_and_without_git_suffix() {
        for u in ["https://github.com/owner/name", "https://github.com/owner/name.git"] {
            assert_eq!(RepoSlug::from_remote_url(u).unwrap().to_string(), "owner/name");
        }
    }

    #[test]
    fn parses_ssh_url_scheme() {
        assert_eq!(
            RepoSlug::from_remote_url("ssh://git@github.com/owner/name.git").unwrap().to_string(),
            "owner/name"
        );
    }

    #[test]
    fn rejects_non_github_remote() {
        let err = RepoSlug::from_remote_url("https://gitlab.com/owner/name.git").unwrap_err();
        assert!(err.to_string().contains("GitHub"));
    }


    #[test]
    fn parses_https_remote_with_credentials() {
        assert_eq!(
            RepoSlug::from_remote_url("https://user:secret@github.com/owner/name.git")
                .unwrap()
                .to_string(),
            "owner/name"
        );
    }

    #[test]
    fn error_does_not_echo_credentials() {
        let err = RepoSlug::from_remote_url("https://user:secret@gitlab.com/owner/name.git")
            .unwrap_err()
            .to_string();
        assert!(!err.contains("secret"), "{err}");
        assert!(err.contains("gitlab.com"), "{err}");
    }
}
