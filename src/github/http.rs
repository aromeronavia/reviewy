//! The reqwest-backed [`PullRequestSource`]: GraphQL for pull requests and
//! conversations, REST for diffs and for posting.

use anyhow::{Context, Result, anyhow, bail};
use reqwest::StatusCode;
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::{
    ChangedFile, ChecksState, Comment, Conversation, DiffSide, FileStatus, PullRequest,
    PullRequestSource, ReviewDecision, ReviewState, ReviewSubmission, Thread,
};
use crate::repo::RepoSlug;

const DEFAULT_BASE_URL: &str = "https://api.github.com";
const PER_PAGE: usize = 100;
const MAX_PAGES: u32 = 10;
const MAX_GRAPHQL_PAGES: u32 = 20;

const LIST_QUERY: &str = r#"query($owner: String!, $name: String!, $after: String) {
  repository(owner: $owner, name: $name) {
    pullRequests(states: OPEN, first: 50, after: $after, orderBy: {field: UPDATED_AT, direction: DESC}) {
      pageInfo { hasNextPage endCursor }
      nodes {
        number title isDraft reviewDecision author { login }
        headRefName baseRefName headRefOid comments { totalCount }
        commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }
      }
    }
  }
}"#;

const CONVERSATION_QUERY: &str = r#"query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      body
      comments(first: 100) { nodes { author { login } body createdAt } }
      reviews(first: 100) { nodes { author { login } state body createdAt } }
      reviewThreads(first: 100) {
        nodes {
          path line diffSide isOutdated isResolved
          comments(first: 50) { nodes { author { login } body createdAt } }
        }
      }
    }
  }
}"#;

pub struct HttpGitHub {
    client: Client,
    repo: RepoSlug,
    base_url: String,
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
        Ok(HttpGitHub {
            client,
            repo,
            base_url: DEFAULT_BASE_URL.to_string(),
        })
    }

    /// Points the client at another host, for tests and GitHub Enterprise.
    pub fn with_base_url(mut self, base_url: String) -> HttpGitHub {
        self.base_url = base_url;
        self
    }

    /// Runs one GraphQL query and returns its `data`.
    fn graphql<T: DeserializeOwned>(&self, query: &str, variables: Value) -> Result<T> {
        let response = self
            .client
            .post(format!("{}/graphql", self.base_url))
            .json(&json!({ "query": query, "variables": variables }))
            .send()
            .map_err(network_error)?;
        let status = response.status();
        if !status.is_success() {
            bail!("{}", describe_status(status));
        }
        let parsed: GraphqlResponse<T> = response.json().context("unexpected response body")?;
        parsed.into_data()
    }

    /// Fetches every page of a REST list endpoint, stopping at the first
    /// short page or after `MAX_PAGES`.
    fn get_all_pages<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<Vec<T>> {
        let mut items = Vec::new();
        for page in 1..=MAX_PAGES {
            let response = self
                .client
                .get(page_url(&self.base_url, path, query, page))
                .send()
                .map_err(network_error)?;
            let status = response.status();
            if !status.is_success() {
                bail!("{}", describe_status(status));
            }
            let batch: Vec<T> = response.json().context("unexpected response body")?;
            let last_page = batch.len() < PER_PAGE;
            items.extend(batch);
            if last_page {
                break;
            }
        }
        Ok(items)
    }
}

/// A short, footer-sized explanation of a non-2xx status. Error text is shown
/// on one terminal row, so the status code comes first and hints stay terse.
fn describe_status(status: StatusCode) -> String {
    let hint = match status {
        StatusCode::UNAUTHORIZED => "; check your token",
        StatusCode::FORBIDDEN => "; rate limit or no access",
        StatusCode::NOT_FOUND => "; check the repository",
        _ => "",
    };
    format!("GitHub returned {status}{hint}")
}

/// Keeps only the root cause of a transport failure; reqwest's own message
/// repeats the URL and would not fit on a terminal row.
fn network_error(error: reqwest::Error) -> anyhow::Error {
    let error = anyhow::Error::from(error.without_url());
    anyhow!("network error: {}", error.root_cause())
}

/// Builds `{base}{path}?{query...}&per_page=N&page=M`. Values are repository
/// slugs and fixed words, so no percent-encoding is needed.
fn page_url(base_url: &str, path: &str, query: &[(&str, &str)], page: u32) -> String {
    let mut params: Vec<String> = query.iter().map(|(k, v)| format!("{k}={v}")).collect();
    params.push(format!("per_page={PER_PAGE}"));
    params.push(format!("page={page}"));
    format!("{base_url}{path}?{}", params.join("&"))
}

impl PullRequestSource for HttpGitHub {
    fn list_open(&self) -> Result<Vec<PullRequest>> {
        let mut prs = Vec::new();
        let mut after: Option<String> = None;
        for _ in 0..MAX_GRAPHQL_PAGES {
            let variables = json!({
                "owner": self.repo.owner,
                "name": self.repo.name,
                "after": after,
            });
            let data: ListData = self
                .graphql(LIST_QUERY, variables)
                .context("pull requests")?;
            let connection = data
                .repository
                .ok_or_else(|| anyhow!("pull requests: no such repository"))?
                .pull_requests;
            prs.extend(connection.nodes.into_iter().map(PullRequest::from));
            if !connection.page_info.has_next_page {
                break;
            }
            after = connection.page_info.end_cursor;
        }
        Ok(prs)
    }

    fn changed_files(&self, number: u64) -> Result<Vec<ChangedFile>> {
        let path = format!(
            "/repos/{}/{}/pulls/{number}/files",
            self.repo.owner, self.repo.name
        );
        let wire: Vec<WireFile> = self
            .get_all_pages(&path, &[])
            .with_context(|| format!("files for #{number}"))?;
        Ok(wire.into_iter().map(ChangedFile::from).collect())
    }

    fn conversation(&self, number: u64) -> Result<Conversation> {
        let variables = json!({
            "owner": self.repo.owner,
            "name": self.repo.name,
            "number": number,
        });
        let data: ConversationData = self
            .graphql(CONVERSATION_QUERY, variables)
            .with_context(|| format!("conversation for #{number}"))?;
        let wire = data
            .repository
            .and_then(|repository| repository.pull_request)
            .ok_or_else(|| anyhow!("conversation for #{number}: no such pull request"))?;
        Ok(conversation_from_wire(wire))
    }

    fn post_comment(&self, _number: u64, _body: &str) -> Result<()> {
        todo!("Task 3")
    }

    fn submit_review(&self, _number: u64, _review: &ReviewSubmission) -> Result<()> {
        todo!("Task 3")
    }
}

// ---- GraphQL envelope -------------------------------------------------------

#[derive(Deserialize)]
struct GraphqlResponse<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Vec<GraphqlError>,
}

#[derive(Deserialize)]
struct GraphqlError {
    message: String,
}

impl<T> GraphqlResponse<T> {
    fn into_data(self) -> Result<T> {
        if let Some(first) = self.errors.first() {
            bail!("GitHub: {}", first.message);
        }
        self.data.ok_or_else(|| anyhow!("GitHub returned no data"))
    }
}

#[derive(Deserialize)]
struct Nodes<T> {
    nodes: Vec<T>,
}

#[derive(Deserialize)]
struct WireUser {
    login: String,
}

fn login(user: Option<WireUser>) -> String {
    user.map(|user| user.login).unwrap_or_else(|| "ghost".into())
}

// ---- pull request list ------------------------------------------------------

#[derive(Deserialize)]
struct ListData {
    repository: Option<ListRepository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListRepository {
    pull_requests: PrConnection,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrConnection {
    page_info: PageInfo,
    nodes: Vec<WirePr>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WirePr {
    number: u64,
    title: String,
    is_draft: bool,
    review_decision: Option<String>,
    author: Option<WireUser>,
    head_ref_name: String,
    base_ref_name: String,
    head_ref_oid: String,
    comments: WireCount,
    commits: Nodes<WireCommitNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireCount {
    total_count: u64,
}

#[derive(Deserialize)]
struct WireCommitNode {
    commit: WireCommit,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireCommit {
    status_check_rollup: Option<WireRollup>,
}

#[derive(Deserialize)]
struct WireRollup {
    state: String,
}

fn review_decision(state: Option<&str>) -> ReviewDecision {
    match state {
        Some("APPROVED") => ReviewDecision::Approved,
        Some("CHANGES_REQUESTED") => ReviewDecision::ChangesRequested,
        Some("REVIEW_REQUIRED") => ReviewDecision::ReviewRequired,
        _ => ReviewDecision::None,
    }
}

fn checks_state(state: Option<&str>) -> ChecksState {
    match state {
        Some("SUCCESS") => ChecksState::Success,
        Some("FAILURE" | "ERROR") => ChecksState::Failure,
        Some("PENDING" | "EXPECTED") => ChecksState::Pending,
        _ => ChecksState::None,
    }
}

impl From<WirePr> for PullRequest {
    fn from(wire: WirePr) -> PullRequest {
        let rollup = wire
            .commits
            .nodes
            .into_iter()
            .next()
            .and_then(|node| node.commit.status_check_rollup)
            .map(|rollup| rollup.state);
        PullRequest {
            number: wire.number,
            title: wire.title,
            author: login(wire.author),
            head_ref: wire.head_ref_name,
            base_ref: wire.base_ref_name,
            head_sha: wire.head_ref_oid,
            draft: wire.is_draft,
            review_decision: review_decision(wire.review_decision.as_deref()),
            checks: checks_state(rollup.as_deref()),
            comment_count: wire.comments.total_count,
        }
    }
}

// ---- conversation -----------------------------------------------------------

#[derive(Deserialize)]
struct ConversationData {
    repository: Option<ConversationRepository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConversationRepository {
    pull_request: Option<WireConversation>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireConversation {
    body: Option<String>,
    comments: Nodes<WireComment>,
    reviews: Nodes<WireReview>,
    review_threads: Nodes<WireThread>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireComment {
    author: Option<WireUser>,
    body: String,
    created_at: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireReview {
    author: Option<WireUser>,
    state: String,
    body: String,
    created_at: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireThread {
    path: String,
    line: Option<u64>,
    diff_side: String,
    is_outdated: bool,
    is_resolved: bool,
    comments: Nodes<WireComment>,
}

fn review_state(state: &str) -> ReviewState {
    match state {
        "APPROVED" => ReviewState::Approved,
        "CHANGES_REQUESTED" => ReviewState::ChangesRequested,
        "DISMISSED" => ReviewState::Dismissed,
        "PENDING" => ReviewState::Pending,
        _ => ReviewState::Commented,
    }
}

fn diff_side(side: &str) -> DiffSide {
    match side {
        "LEFT" => DiffSide::Left,
        _ => DiffSide::Right,
    }
}

impl From<WireComment> for Comment {
    fn from(wire: WireComment) -> Comment {
        Comment {
            author: login(wire.author),
            created_at: wire.created_at,
            body: wire.body,
            review_state: None,
        }
    }
}

/// Merges plain comments and review summaries into one timeline. A review
/// that only carried line comments has nothing to say here and is skipped.
fn conversation_from_wire(wire: WireConversation) -> Conversation {
    let mut comments: Vec<Comment> = wire.comments.nodes.into_iter().map(Comment::from).collect();
    for review in wire.reviews.nodes {
        let state = review_state(&review.state);
        let silent = matches!(state, ReviewState::Commented | ReviewState::Pending);
        if silent && review.body.trim().is_empty() {
            continue;
        }
        comments.push(Comment {
            author: login(review.author),
            created_at: review.created_at,
            body: review.body,
            review_state: Some(state),
        });
    }
    comments.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    let threads = wire
        .review_threads
        .nodes
        .into_iter()
        .map(|thread| Thread {
            path: thread.path,
            line: thread.line,
            side: diff_side(&thread.diff_side),
            outdated: thread.is_outdated,
            resolved: thread.is_resolved,
            comments: thread.comments.nodes.into_iter().map(Comment::from).collect(),
        })
        .collect();
    Conversation {
        body: wire.body.unwrap_or_default(),
        comments,
        threads,
    }
}

// ---- REST files -------------------------------------------------------------

#[derive(Deserialize)]
struct WireFile {
    filename: String,
    status: String,
    additions: u64,
    deletions: u64,
    #[serde(default)]
    patch: Option<String>,
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
            page_url("https://api.github.com", "/repos/o/n/pulls", &[("state", "open")], 2),
            "https://api.github.com/repos/o/n/pulls?state=open&per_page=100&page=2"
        );
        assert_eq!(
            page_url("https://api.github.com", "/repos/o/n/pulls/7/files", &[], 1),
            "https://api.github.com/repos/o/n/pulls/7/files?per_page=100&page=1"
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

    const LIST_NODE: &str = r#"{"number":5,"title":"T","isDraft":true,"reviewDecision":"APPROVED","author":{"login":"ann"},"headRefName":"feat","baseRefName":"main","headRefOid":"abc123","comments":{"totalCount":2},"commits":{"nodes":[{"commit":{"statusCheckRollup":{"state":"SUCCESS"}}}]}}"#;
    const NULL_NODE: &str = r#"{"number":6,"title":"U","isDraft":false,"reviewDecision":null,"author":null,"headRefName":"f","baseRefName":"m","headRefOid":"def","comments":{"totalCount":0},"commits":{"nodes":[{"commit":{"statusCheckRollup":null}}]}}"#;
    const CONVERSATION: &str = r#"{"body":"Hello","comments":{"nodes":[{"author":{"login":"bob"},"body":"second","createdAt":"2026-01-03T00:00:00Z"}]},"reviews":{"nodes":[{"author":{"login":"ann"},"state":"APPROVED","body":"lgtm","createdAt":"2026-01-02T00:00:00Z"},{"author":{"login":"cat"},"state":"COMMENTED","body":"","createdAt":"2026-01-01T00:00:00Z"}]},"reviewThreads":{"nodes":[{"path":"src/a.rs","line":7,"diffSide":"LEFT","isOutdated":false,"isResolved":false,"comments":{"nodes":[{"author":{"login":"ann"},"body":"why?","createdAt":"2026-01-02T00:00:00Z"}]}}]}}"#;

    #[test]
    fn maps_list_node_to_pull_request() {
        let wire: WirePr = serde_json::from_str(LIST_NODE).unwrap();
        assert_eq!(
            PullRequest::from(wire),
            PullRequest {
                number: 5,
                title: "T".into(),
                author: "ann".into(),
                head_ref: "feat".into(),
                base_ref: "main".into(),
                head_sha: "abc123".into(),
                draft: true,
                review_decision: ReviewDecision::Approved,
                checks: ChecksState::Success,
                comment_count: 2,
            }
        );
    }

    #[test]
    fn maps_null_decision_checks_and_author() {
        let pr = PullRequest::from(serde_json::from_str::<WirePr>(NULL_NODE).unwrap());
        assert_eq!(pr.review_decision, ReviewDecision::None);
        assert_eq!(pr.checks, ChecksState::None);
        assert_eq!(pr.author, "ghost");
    }

    #[test]
    fn maps_conversation_with_threads() {
        let wire: WireConversation = serde_json::from_str(CONVERSATION).unwrap();
        let conversation = conversation_from_wire(wire);
        assert_eq!(conversation.body, "Hello");
        let authors: Vec<_> = conversation.comments.iter().map(|c| c.author.as_str()).collect();
        assert_eq!(authors, ["ann", "bob"]);
        assert_eq!(conversation.comments[0].review_state, Some(ReviewState::Approved));
        assert_eq!(conversation.comments[1].review_state, None);
        assert_eq!(conversation.threads.len(), 1);
        let thread = &conversation.threads[0];
        assert_eq!((thread.path.as_str(), thread.line, thread.side), ("src/a.rs", Some(7), DiffSide::Left));
        assert!(!thread.outdated);
        assert_eq!(thread.comments[0].body, "why?");
    }

    #[test]
    fn drops_empty_commented_reviews() {
        let wire: WireConversation = serde_json::from_str(CONVERSATION).unwrap();
        let conversation = conversation_from_wire(wire);
        assert!(conversation.comments.iter().all(|c| c.author != "cat"));
    }

    #[test]
    fn graphql_errors_become_errors() {
        let parsed: GraphqlResponse<serde_json::Value> =
            serde_json::from_str(r#"{"errors":[{"message":"boom"}]}"#).unwrap();
        let err = parsed.into_data().unwrap_err();
        assert!(err.to_string().contains("boom"), "{err}");
    }

    #[test]
    #[ignore = "hits api.github.com; run with --ignored"]
    fn live_list_open_pull_requests() {
        let repo = RepoSlug::parse("ratatui/ratatui").unwrap();
        let token = crate::auth::resolve_token().unwrap();
        let prs = HttpGitHub::new(repo, token).unwrap().list_open().unwrap();
        assert!(!prs.is_empty());
        assert!(!prs[0].head_sha.is_empty());
    }

    #[test]
    #[ignore = "hits api.github.com; run with --ignored"]
    fn live_conversation_has_threads() {
        let repo = RepoSlug::parse("ratatui/ratatui").unwrap();
        let token = crate::auth::resolve_token().unwrap();
        let conversation = HttpGitHub::new(repo, token).unwrap().conversation(2266).unwrap();
        assert!(!conversation.threads.is_empty());
        assert!(!conversation.body.is_empty());
    }
}
