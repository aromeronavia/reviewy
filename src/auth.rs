use std::env;
use std::process::Command;

use anyhow::{Result, bail};

/// Finds a GitHub token: `GITHUB_TOKEN`, then `GH_TOKEN`, then `gh auth token`.
pub fn resolve_token() -> Result<String> {
    let from_env = ["GITHUB_TOKEN", "GH_TOKEN"]
        .iter()
        .filter_map(|name| env::var(name).ok())
        .find_map(|raw| clean_token(&raw));
    if let Some(token) = from_env {
        return Ok(token);
    }
    if let Some(token) = token_from_gh_cli() {
        return Ok(token);
    }
    bail!("No GitHub token found. Set GITHUB_TOKEN or run `gh auth login`.")
}

fn token_from_gh_cli() -> Option<String> {
    let output = Command::new("gh").args(["auth", "token"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    clean_token(&String::from_utf8_lossy(&output.stdout))
}

fn clean_token(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_token_trims_whitespace() {
        assert_eq!(clean_token("  gho_abc\n").as_deref(), Some("gho_abc"));
    }

    #[test]
    fn clean_token_rejects_blank() {
        assert_eq!(clean_token(" \n"), None);
    }
}
