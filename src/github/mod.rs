pub mod http;

use anyhow::Result;

/// An open pull request as the UI needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub head_ref: String,
    pub base_ref: String,
    pub draft: bool,
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

/// Where pull requests come from. The UI and worker depend only on this.
pub trait PullRequestSource: Send + Sync {
    fn list_open(&self) -> Result<Vec<PullRequest>>;
    fn changed_files(&self, number: u64) -> Result<Vec<ChangedFile>>;
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
