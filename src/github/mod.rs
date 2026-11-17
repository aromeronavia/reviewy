pub mod http;

use anyhow::Result;

/// An open pull request as the sidebar and the review flow need it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub head_ref: String,
    pub base_ref: String,
    /// Head commit at the time of listing; line comments are pinned to it.
    pub head_sha: String,
    pub draft: bool,
    pub review_decision: ReviewDecision,
    pub checks: ChecksState,
    pub comment_count: u64,
}

/// GitHub's overall review decision for a pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewDecision {
    Approved,
    ChangesRequested,
    ReviewRequired,
    None,
}

/// The rolled-up state of CI checks on the head commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChecksState {
    Success,
    Failure,
    Pending,
    None,
}

/// The verdict a submitted review carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewState {
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
    Pending,
}

/// Which side of a diff a line comment is attached to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffSide {
    Left,
    Right,
}

/// The verdict to submit with a review.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewEvent {
    Approve,
    RequestChanges,
    Comment,
}

/// A pull request comment or a review's summary, in the conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub author: String,
    pub created_at: String,
    pub body: String,
    /// `Some` when this entry is a review rather than a plain comment.
    pub review_state: Option<ReviewState>,
}

/// A review thread anchored to a line of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread {
    pub path: String,
    /// `None` when the thread no longer maps onto the current diff.
    pub line: Option<u64>,
    pub side: DiffSide,
    pub outdated: bool,
    pub resolved: bool,
    pub comments: Vec<Comment>,
}

/// Everything about a pull request that is not its diff.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Conversation {
    pub body: String,
    pub comments: Vec<Comment>,
    pub threads: Vec<Thread>,
    /// Older comments, reviews, and threads that were not fetched.
    pub omitted: u64,
}

/// A line comment the user wrote locally, waiting to go with a review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewLineComment {
    pub path: String,
    pub line: u64,
    pub side: DiffSide,
    pub body: String,
}

/// A review ready to be posted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewSubmission {
    pub event: ReviewEvent,
    pub body: String,
    pub commit_sha: String,
    pub comments: Vec<NewLineComment>,
}

/// What happened to a file in a pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Modified,
    Removed,
    Renamed,
    Other,
}

impl FileStatus {
    /// Maps the `status` string from the GitHub files endpoint.
    pub fn from_api(status: &str) -> FileStatus {
        match status {
            "added" => FileStatus::Added,
            "modified" => FileStatus::Modified,
            "removed" => FileStatus::Removed,
            "renamed" => FileStatus::Renamed,
            _ => FileStatus::Other,
        }
    }

    /// One-letter marker shown next to a file name.
    pub fn letter(self) -> char {
        match self {
            FileStatus::Added => 'A',
            FileStatus::Modified => 'M',
            FileStatus::Removed => 'D',
            FileStatus::Renamed => 'R',
            FileStatus::Other => '?',
        }
    }
}

/// One file touched by a pull request. `patch` is `None` for binary or
/// oversized files, where GitHub omits it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub filename: String,
    pub status: FileStatus,
    pub additions: u64,
    pub deletions: u64,
    pub patch: Option<String>,
}

/// Where pull requests come from and where reviews go. The UI and worker
/// depend only on this.
pub trait PullRequestSource: Send + Sync {
    fn list_open(&self) -> Result<Vec<PullRequest>>;
    fn changed_files(&self, number: u64) -> Result<Vec<ChangedFile>>;
    fn conversation(&self, number: u64) -> Result<Conversation>;
    fn post_comment(&self, number: u64, body: &str) -> Result<()>;
    fn submit_review(&self, number: u64, review: &ReviewSubmission) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_api_status_strings() {
        assert_eq!(FileStatus::from_api("added"), FileStatus::Added);
        assert_eq!(FileStatus::from_api("modified"), FileStatus::Modified);
        assert_eq!(FileStatus::from_api("removed"), FileStatus::Removed);
        assert_eq!(FileStatus::from_api("renamed"), FileStatus::Renamed);
        assert_eq!(FileStatus::from_api("copied"), FileStatus::Other);
    }

    #[test]
    fn status_letters() {
        assert_eq!(FileStatus::Added.letter(), 'A');
        assert_eq!(FileStatus::Modified.letter(), 'M');
        assert_eq!(FileStatus::Removed.letter(), 'D');
        assert_eq!(FileStatus::Renamed.letter(), 'R');
        assert_eq!(FileStatus::Other.letter(), '?');
    }
}
