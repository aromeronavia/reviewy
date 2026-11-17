//! The reqwest-backed [`PullRequestSource`] against the GitHub REST API.

use anyhow::{Context, Result};
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::{ChangedFile, FileStatus, PullRequest, PullRequestSource};
use crate::repo::RepoSlug;

const BASE_URL: &str = "https://api.github.com";
const PER_PAGE: usize = 100;
const MAX_PAGES: u32 = 10;

pub struct HttpGitHub {
    client: Client,
    repo: RepoSlug,
}

impl HttpGitHub {
    pub fn new(repo: RepoSlug, token: String) -> Result<HttpGitHub> {
        let mut auth = HeaderValue::from_str(&format!("Bearer {token}"))
            .context("the GitHub token contains characters that are not valid in a header")?;
        auth.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, auth);
        headers.insert(ACCEPT, HeaderValue::from_static("application/vnd.github+json"));
        headers.insert("X-GitHub-Api-Version", HeaderValue::from_static("2022-11-28"));
        let client = Client::builder()
            .user_agent("reviewy")
            .default_headers(headers)
            .build()
            .context("failed to build the HTTP client")?;
        Ok(HttpGitHub { client, repo })
    }

    /// Fetches every page of a list endpoint, stopping at the first short
    /// page or after `MAX_PAGES`.
    fn get_all_pages<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<Vec<T>> {
        let mut items = Vec::new();
        for page in 1..=MAX_PAGES {
            let batch: Vec<T> = self
                .client
                .get(page_url(path, query, page))
                .send()?
                .error_for_status()?
                .json()?;
            let last_page = batch.len() < PER_PAGE;
            items.extend(batch);
            if last_page {
                break;
            }
        }
        Ok(items)
    }
}

/// Builds `{BASE_URL}{path}?{query...}&per_page=N&page=M`. Values are
/// repository slugs and fixed words, so no percent-encoding is needed.
fn page_url(path: &str, query: &[(&str, &str)], page: u32) -> String {
    let mut params: Vec<String> = query.iter().map(|(k, v)| format!("{k}={v}")).collect();
    params.push(format!("per_page={PER_PAGE}"));
    params.push(format!("page={page}"));
    format!("{BASE_URL}{path}?{}", params.join("&"))
}

impl PullRequestSource for HttpGitHub {
    fn list_open(&self) -> Result<Vec<PullRequest>> {
        let path = format!("/repos/{}/{}/pulls", self.repo.owner, self.repo.name);
        let query = [("state", "open"), ("sort", "updated"), ("direction", "desc")];
        let wire: Vec<WirePullRequest> = self
            .get_all_pages(&path, &query)
            .with_context(|| format!("listing pull requests for {}", self.repo))?;
        Ok(wire.into_iter().map(PullRequest::from).collect())
    }

    fn changed_files(&self, number: u64) -> Result<Vec<ChangedFile>> {
        let path = format!(
            "/repos/{}/{}/pulls/{number}/files",
            self.repo.owner, self.repo.name
        );
        let wire: Vec<WireFile> = self
            .get_all_pages(&path, &[])
            .with_context(|| format!("loading files for #{number}"))?;
        Ok(wire.into_iter().map(ChangedFile::from).collect())
    }
}

#[derive(Deserialize)]
struct WirePullRequest {
    number: u64,
    title: String,
    #[serde(default)]
    draft: bool,
    user: Option<WireUser>,
    head: WireRef,
    base: WireRef,
}

#[derive(Deserialize)]
struct WireUser {
    login: String,
}

#[derive(Deserialize)]
struct WireRef {
    #[serde(rename = "ref")]
    name: String,
}

#[derive(Deserialize)]
struct WireFile {
    filename: String,
    status: String,
    additions: u64,
    deletions: u64,
    #[serde(default)]
    patch: Option<String>,
}

impl From<WirePullRequest> for PullRequest {
    fn from(wire: WirePullRequest) -> PullRequest {
        PullRequest {
            number: wire.number,
            title: wire.title,
            author: wire.user.map(|user| user.login).unwrap_or_else(|| "ghost".into()),
            head_ref: wire.head.name,
            base_ref: wire.base.name,
            draft: wire.draft,
        }
    }
}

impl From<WireFile> for ChangedFile {
    fn from(wire: WireFile) -> ChangedFile {
        ChangedFile {
            filename: wire.filename,
            status: FileStatus::from_api(&wire.status),
            additions: wire.additions,
            deletions: wire.deletions,
            patch: wire.patch,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::FileStatus;

    #[test]
    fn builds_paged_urls() {
        assert_eq!(
            page_url("/repos/o/n/pulls", &[("state", "open")], 2),
            "https://api.github.com/repos/o/n/pulls?state=open&per_page=100&page=2"
        );
        assert_eq!(
            page_url("/repos/o/n/pulls/7/files", &[], 1),
            "https://api.github.com/repos/o/n/pulls/7/files?per_page=100&page=1"
        );
    }

    #[test]
    fn converts_wire_pull_request() {
        let json = r#"{"number":5,"title":"T","user":{"login":"ann"},"head":{"ref":"feat"},"base":{"ref":"main"}}"#;
        let wire: WirePullRequest = serde_json::from_str(json).unwrap();
        assert_eq!(
            PullRequest::from(wire),
            PullRequest {
                number: 5,
                title: "T".into(),
                author: "ann".into(),
                head_ref: "feat".into(),
                base_ref: "main".into(),
                draft: false,
            }
        );
    }

    #[test]
    fn converts_wire_file_without_patch() {
        let json = r#"{"filename":"old.rs","status":"removed","additions":0,"deletions":12}"#;
        let wire: WireFile = serde_json::from_str(json).unwrap();
        let file = ChangedFile::from(wire);
        assert_eq!(file.status, FileStatus::Removed);
        assert_eq!(file.patch, None);
        assert_eq!(file.deletions, 12);
    }

    #[test]
    #[ignore = "hits api.github.com; run with --ignored"]
    fn live_list_open_pull_requests() {
        let repo = RepoSlug::parse("ratatui/ratatui").unwrap();
        let token = crate::auth::resolve_token().unwrap();
        let prs = HttpGitHub::new(repo, token).unwrap().list_open().unwrap();
        assert!(!prs.is_empty());
    }
}
