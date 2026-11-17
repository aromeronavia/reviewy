//! All application state and the rules for changing it. No I/O, no
//! terminal types: the keymap turns keys into [`Action`]s, the worker turns
//! [`Request`]s into [`Response`]s, and the UI draws whatever is here.

use crate::diff::{DiffLine, parse_patch};
use crate::github::{ChangedFile, Conversation, PullRequest, ReviewSubmission};
use crate::repo::RepoSlug;

/// Which pane receives navigation keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    PullRequests,
    Files,
    Diff,
}

impl Focus {
    fn next(self) -> Focus {
        match self {
            Focus::PullRequests => Focus::Files,
            Focus::Files => Focus::Diff,
            Focus::Diff => Focus::PullRequests,
        }
    }

    fn prev(self) -> Focus {
        match self {
            Focus::PullRequests => Focus::Diff,
            Focus::Files => Focus::PullRequests,
            Focus::Diff => Focus::Files,
        }
    }
}

/// Something the user asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    NextPane,
    PrevPane,
    Up,
    Down,
    Top,
    Bottom,
    PageUp,
    PageDown,
    Open,
    Refresh,
}

/// Work for the background worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    LoadPullRequests,
    LoadFiles { number: u64 },
    LoadConversation { number: u64 },
    PostComment { number: u64, body: String },
    SubmitReview { number: u64, review: ReviewSubmission },
}

/// What the background worker sends back.
#[derive(Debug)]
pub enum Response {
    PullRequests(Result<Vec<PullRequest>, String>),
    Files {
        number: u64,
        result: Result<Vec<ChangedFile>, String>,
    },
    Conversation {
        number: u64,
        result: Result<Conversation, String>,
    },
    CommentPosted {
        number: u64,
        result: Result<(), String>,
    },
    ReviewSubmitted {
        number: u64,
        result: Result<(), String>,
    },
}

/// What the footer should say about in-flight work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Idle,
    Loading(String),
    Error(String),
}

/// Lines moved by `PageUp`/`PageDown`.
pub const PAGE: usize = 20;

const LOADING_PULL_REQUESTS: &str = "Loading pull requests…";

fn loading_files_message(number: u64) -> String {
    format!("Loading #{number}…")
}

pub struct App {
    pub repo: RepoSlug,
    pub focus: Focus,
    pub prs: Vec<PullRequest>,
    pub pr_index: usize,
    pub files: Vec<ChangedFile>,
    pub file_index: usize,
    /// The PR whose files are currently shown.
    pub loaded_pr: Option<u64>,
    /// The PR whose files have been requested but not yet received.
    pub pending_pr: Option<u64>,
    pub diff: Vec<DiffLine>,
    pub diff_scroll: usize,
    pub status: Status,
    pub should_quit: bool,
}

/// A cursor movement inside a list or a scrollable text.
#[derive(Clone, Copy)]
enum Move {
    Up,
    Down,
    Top,
    Bottom,
    PageUp,
    PageDown,
}

fn moved(index: usize, len: usize, mv: Move) -> usize {
    let last = len.saturating_sub(1);
    match mv {
        Move::Up => index.saturating_sub(1),
        Move::Down => (index + 1).min(last),
        Move::Top => 0,
        Move::Bottom => last,
        Move::PageUp => index.saturating_sub(PAGE),
        Move::PageDown => (index + PAGE).min(last),
    }
}

impl App {
    pub fn new(repo: RepoSlug) -> App {
        App {
            repo,
            focus: Focus::PullRequests,
            prs: Vec::new(),
            pr_index: 0,
            files: Vec::new(),
            file_index: 0,
            loaded_pr: None,
            pending_pr: None,
            diff: Vec::new(),
            diff_scroll: 0,
            status: Status::Loading(LOADING_PULL_REQUESTS.into()),
            should_quit: false,
        }
    }

    pub fn selected_pr(&self) -> Option<&PullRequest> {
        self.prs.get(self.pr_index)
    }

    pub fn selected_file(&self) -> Option<&ChangedFile> {
        self.files.get(self.file_index)
    }

    /// Applies a user action; returns work for the worker, if any.
    pub fn dispatch(&mut self, action: Action) -> Option<Request> {
        match action {
            Action::Quit => {
                self.should_quit = true;
                None
            }
            Action::NextPane => {
                self.focus = self.focus.next();
                None
            }
            Action::PrevPane => {
                self.focus = self.focus.prev();
                None
            }
            Action::Up => self.navigate(Move::Up),
            Action::Down => self.navigate(Move::Down),
            Action::Top => self.navigate(Move::Top),
            Action::Bottom => self.navigate(Move::Bottom),
            Action::PageUp => self.page(Move::PageUp),
            Action::PageDown => self.page(Move::PageDown),
            Action::Open => self.open(),
            Action::Refresh => {
                self.status = Status::Loading(LOADING_PULL_REQUESTS.into());
                Some(Request::LoadPullRequests)
            }
        }
    }

    /// Absorbs a worker response. Responses for a PR the user no longer
    /// waits on are dropped.
    pub fn apply(&mut self, response: Response) {
        match response {
            Response::PullRequests(Ok(prs)) => {
                let previous = self.selected_pr().map(|pr| pr.number);
                self.prs = prs;
                self.pr_index = previous
                    .and_then(|number| self.prs.iter().position(|pr| pr.number == number))
                    .unwrap_or(0);
                self.status = Status::Idle;
            }
            Response::PullRequests(Err(message)) => self.status = Status::Error(message),
            Response::Files { number, result } if self.pending_pr == Some(number) => {
                self.pending_pr = None;
                match result {
                    Ok(files) => {
                        self.files = files;
                        self.loaded_pr = Some(number);
                        self.select_file(0);
                        self.status = Status::Idle;
                    }
                    Err(message) => self.status = Status::Error(message),
                }
            }
            Response::Files { .. }
            | Response::Conversation { .. }
            | Response::CommentPosted { .. }
            | Response::ReviewSubmitted { .. } => {}
        }
    }

    fn navigate(&mut self, mv: Move) -> Option<Request> {
        match self.focus {
            Focus::PullRequests => self.pr_index = moved(self.pr_index, self.prs.len(), mv),
            Focus::Files => {
                let next = moved(self.file_index, self.files.len(), mv);
                if next != self.file_index {
                    self.select_file(next);
                }
            }
            Focus::Diff => self.diff_scroll = moved(self.diff_scroll, self.diff.len(), mv),
        }
        None
    }

    fn page(&mut self, mv: Move) -> Option<Request> {
        if self.focus == Focus::Diff {
            self.diff_scroll = moved(self.diff_scroll, self.diff.len(), mv);
        }
        None
    }

    fn open(&mut self) -> Option<Request> {
        match self.focus {
            Focus::PullRequests => {
                let number = self.selected_pr()?.number;
                self.focus = Focus::Files;
                if self.loaded_pr == Some(number) {
                    self.cancel_pending_files();
                    return None;
                }
                if self.pending_pr == Some(number) {
                    return None;
                }
                self.pending_pr = Some(number);
                self.status = Status::Loading(loading_files_message(number));
                Some(Request::LoadFiles { number })
            }
            Focus::Files => {
                self.focus = Focus::Diff;
                None
            }
            Focus::Diff => None,
        }
    }

    /// Forgets a files request the user is no longer waiting for, so its
    /// response cannot replace the PR they went back to.
    fn cancel_pending_files(&mut self) {
        if let Some(pending) = self.pending_pr.take()
            && self.status == Status::Loading(loading_files_message(pending))
        {
            self.status = Status::Idle;
        }
    }

    fn select_file(&mut self, index: usize) {
        self.file_index = index;
        self.diff = self
            .selected_file()
            .and_then(|file| file.patch.as_deref())
            .map(parse_patch)
            .unwrap_or_default();
        self.diff_scroll = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::DiffLineKind;
    use crate::github::{ChecksState, FileStatus, ReviewDecision};

    fn repo() -> RepoSlug {
        RepoSlug::parse("owner/name").unwrap()
    }

    fn pr(n: u64) -> PullRequest {
        PullRequest {
            number: n,
            title: format!("PR {n}"),
            author: "ann".into(),
            head_ref: "feat".into(),
            base_ref: "main".into(),
            head_sha: format!("sha{n}"),
            draft: false,
            review_decision: ReviewDecision::None,
            checks: ChecksState::None,
            comment_count: 0,
        }
    }

    fn file(name: &str, patch: Option<&str>) -> ChangedFile {
        ChangedFile {
            filename: name.into(),
            status: FileStatus::Modified,
            additions: 1,
            deletions: 1,
            patch: patch.map(str::to_string),
        }
    }

    /// An app with PRs #1 and #2 loaded and #2 selected and opened.
    fn app_with_pr2_opened() -> App {
        let mut app = App::new(repo());
        app.apply(Response::PullRequests(Ok(vec![pr(1), pr(2)])));
        app.dispatch(Action::Down);
        assert_eq!(app.dispatch(Action::Open), Some(Request::LoadFiles { number: 2 }));
        app
    }

    fn two_files() -> Vec<ChangedFile> {
        vec![
            file("a.rs", Some("@@ -1 +1 @@\n-x\n+y")),
            file("b.rs", Some("+z")),
        ]
    }

    #[test]
    fn starts_loading_with_pr_pane_focused() {
        let app = App::new(repo());
        assert_eq!(app.focus, Focus::PullRequests);
        assert_eq!(app.status, Status::Loading("Loading pull requests…".into()));
        assert!(app.prs.is_empty());
        assert!(!app.should_quit);
    }

    #[test]
    fn pull_requests_response_populates_list_and_clears_status() {
        let mut app = App::new(repo());
        app.apply(Response::PullRequests(Ok(vec![pr(1), pr(2)])));
        assert_eq!(app.prs.len(), 2);
        assert_eq!(app.pr_index, 0);
        assert_eq!(app.status, Status::Idle);
    }

    #[test]
    fn pull_requests_error_sets_status() {
        let mut app = App::new(repo());
        app.apply(Response::PullRequests(Err("boom".into())));
        assert_eq!(app.status, Status::Error("boom".into()));
    }

    #[test]
    fn down_and_up_clamp_within_pr_list() {
        let mut app = App::new(repo());
        app.apply(Response::PullRequests(Ok(vec![pr(1), pr(2)])));
        app.dispatch(Action::Down);
        app.dispatch(Action::Down);
        assert_eq!(app.pr_index, 1);
        app.dispatch(Action::Up);
        app.dispatch(Action::Up);
        assert_eq!(app.pr_index, 0);
    }

    #[test]
    fn open_requests_files_for_selected_pr() {
        let app = app_with_pr2_opened();
        assert_eq!(app.focus, Focus::Files);
        assert_eq!(app.status, Status::Loading("Loading #2…".into()));
        assert_eq!(app.pending_pr, Some(2));
    }

    #[test]
    fn open_on_empty_list_is_noop() {
        let mut app = App::new(repo());
        app.apply(Response::PullRequests(Ok(vec![])));
        assert_eq!(app.dispatch(Action::Open), None);
        assert_eq!(app.focus, Focus::PullRequests);
        app.dispatch(Action::Down);
        app.dispatch(Action::Bottom);
        assert_eq!(app.pr_index, 0);
    }

    #[test]
    fn files_response_sets_files_and_diff() {
        let mut app = app_with_pr2_opened();
        app.apply(Response::Files { number: 2, result: Ok(two_files()) });
        assert_eq!(app.files.len(), 2);
        assert_eq!(app.file_index, 0);
        assert_eq!(app.loaded_pr, Some(2));
        assert_eq!(app.pending_pr, None);
        assert_eq!(app.diff.len(), 3);
        assert_eq!(app.diff[1].kind, DiffLineKind::Removed);
        assert_eq!(app.status, Status::Idle);
    }

    #[test]
    fn files_error_sets_status_and_clears_pending() {
        let mut app = app_with_pr2_opened();
        app.apply(Response::Files { number: 2, result: Err("nope".into()) });
        assert_eq!(app.status, Status::Error("nope".into()));
        assert_eq!(app.pending_pr, None);
    }

    #[test]
    fn stale_files_response_is_ignored() {
        let mut app = app_with_pr2_opened();
        app.apply(Response::Files { number: 1, result: Ok(two_files()) });
        assert!(app.files.is_empty());
        assert_eq!(app.pending_pr, Some(2));
        assert_eq!(app.status, Status::Loading("Loading #2…".into()));
    }

    #[test]
    fn files_with_no_patch_give_empty_diff() {
        let mut app = app_with_pr2_opened();
        app.apply(Response::Files { number: 2, result: Ok(vec![file("bin.png", None)]) });
        assert!(app.diff.is_empty());
        assert_eq!(app.loaded_pr, Some(2));
    }

    #[test]
    fn open_on_loaded_pr_only_moves_focus() {
        let mut app = app_with_pr2_opened();
        app.apply(Response::Files { number: 2, result: Ok(two_files()) });
        app.dispatch(Action::PrevPane);
        assert_eq!(app.focus, Focus::PullRequests);
        assert_eq!(app.dispatch(Action::Open), None);
        assert_eq!(app.focus, Focus::Files);
    }

    #[test]
    fn open_in_files_focuses_diff() {
        let mut app = app_with_pr2_opened();
        app.apply(Response::Files { number: 2, result: Ok(two_files()) });
        assert_eq!(app.dispatch(Action::Open), None);
        assert_eq!(app.focus, Focus::Diff);
    }

    #[test]
    fn moving_in_files_reparses_diff_and_resets_scroll() {
        let mut app = app_with_pr2_opened();
        app.apply(Response::Files { number: 2, result: Ok(two_files()) });
        app.dispatch(Action::NextPane);
        app.dispatch(Action::Down);
        assert_eq!(app.diff_scroll, 1);
        app.dispatch(Action::PrevPane);
        app.dispatch(Action::Down);
        assert_eq!(app.file_index, 1);
        assert_eq!(app.diff.len(), 1);
        assert_eq!(app.diff[0].text, "+z");
        assert_eq!(app.diff_scroll, 0);
    }

    #[test]
    fn diff_scroll_clamps_and_pages() {
        let mut app = app_with_pr2_opened();
        let patch = "+l\n".repeat(30);
        app.apply(Response::Files { number: 2, result: Ok(vec![file("big.rs", Some(&patch))]) });
        app.dispatch(Action::NextPane);
        assert_eq!(app.focus, Focus::Diff);
        app.dispatch(Action::Up);
        assert_eq!(app.diff_scroll, 0);
        app.dispatch(Action::Down);
        assert_eq!(app.diff_scroll, 1);
        app.dispatch(Action::PageDown);
        assert_eq!(app.diff_scroll, 21);
        app.dispatch(Action::PageDown);
        assert_eq!(app.diff_scroll, 29);
        app.dispatch(Action::Top);
        assert_eq!(app.diff_scroll, 0);
        app.dispatch(Action::Bottom);
        assert_eq!(app.diff_scroll, 29);
        app.dispatch(Action::PageUp);
        assert_eq!(app.diff_scroll, 9);
    }

    #[test]
    fn page_keys_do_nothing_outside_diff() {
        let mut app = App::new(repo());
        app.apply(Response::PullRequests(Ok(vec![pr(1), pr(2)])));
        app.dispatch(Action::PageDown);
        assert_eq!(app.pr_index, 0);
    }

    #[test]
    fn panes_cycle_and_wrap() {
        let mut app = App::new(repo());
        app.dispatch(Action::NextPane);
        assert_eq!(app.focus, Focus::Files);
        app.dispatch(Action::NextPane);
        assert_eq!(app.focus, Focus::Diff);
        app.dispatch(Action::NextPane);
        assert_eq!(app.focus, Focus::PullRequests);
        app.dispatch(Action::PrevPane);
        assert_eq!(app.focus, Focus::Diff);
    }

    #[test]
    fn refresh_requests_reload_and_keeps_selection() {
        let mut app = App::new(repo());
        app.apply(Response::PullRequests(Ok(vec![pr(1), pr(2)])));
        app.dispatch(Action::Down);
        assert_eq!(app.dispatch(Action::Refresh), Some(Request::LoadPullRequests));
        assert_eq!(app.status, Status::Loading("Loading pull requests…".into()));
        app.apply(Response::PullRequests(Ok(vec![pr(3), pr(2)])));
        assert_eq!(app.pr_index, 1);
        assert_eq!(app.selected_pr().map(|p| p.number), Some(2));
    }

    #[test]
    fn quit_sets_flag() {
        let mut app = App::new(repo());
        app.dispatch(Action::Quit);
        assert!(app.should_quit);
    }


    #[test]
    fn reopening_loaded_pr_cancels_pending_request() {
        let mut app = app_with_pr2_opened();
        app.apply(Response::Files { number: 2, result: Ok(two_files()) });
        app.dispatch(Action::PrevPane);
        app.dispatch(Action::Up);
        assert_eq!(app.dispatch(Action::Open), Some(Request::LoadFiles { number: 1 }));
        app.dispatch(Action::PrevPane);
        app.dispatch(Action::Down);
        assert_eq!(app.dispatch(Action::Open), None);
        assert_eq!(app.focus, Focus::Files);
        assert_eq!(app.pending_pr, None);
        assert_eq!(app.status, Status::Idle);
        app.apply(Response::Files { number: 1, result: Ok(vec![file("other.rs", Some("+o"))]) });
        assert_eq!(app.loaded_pr, Some(2));
        assert_eq!(app.files, two_files());
    }

    #[test]
    fn reopening_loaded_pr_keeps_a_pull_request_reload_status() {
        let mut app = app_with_pr2_opened();
        app.apply(Response::Files { number: 2, result: Ok(two_files()) });
        app.dispatch(Action::PrevPane);
        app.dispatch(Action::Refresh);
        app.dispatch(Action::Open);
        assert_eq!(app.status, Status::Loading("Loading pull requests…".into()));
    }
}
