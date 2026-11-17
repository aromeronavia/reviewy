//! All application state and the rules for changing it. No I/O, no
//! terminal types: the keymap turns keys into [`Action`]s, the worker turns
//! [`Request`]s into [`Response`]s, and the UI draws whatever is here.

use crate::diff::{DiffLine, DiffLineKind, parse_patch};
use crate::github::{
    ChangedFile, Conversation, DiffSide, NewLineComment, PullRequest, ReviewEvent, ReviewState,
    ReviewSubmission,
};
use crate::repo::RepoSlug;

/// Which pane receives navigation keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    PullRequests,
    Files,
    Diff,
    Conversation,
}

/// What the right side shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Changes,
    Conversation,
}

impl Tab {
    fn panes(self) -> &'static [Focus] {
        match self {
            Tab::Changes => &[Focus::PullRequests, Focus::Files, Focus::Diff],
            Tab::Conversation => &[Focus::PullRequests, Focus::Conversation],
        }
    }

    fn first_pane(self) -> Focus {
        match self {
            Tab::Changes => Focus::Files,
            Tab::Conversation => Focus::Conversation,
        }
    }

    fn other(self) -> Tab {
        match self {
            Tab::Changes => Tab::Conversation,
            Tab::Conversation => Tab::Changes,
        }
    }
}

/// What keys mean right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Browse,
    PickReviewEvent,
    Compose(Compose),
}

/// Text being written in the overlay box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compose {
    pub target: ComposeTarget,
    pub text: String,
}

/// Where a composed text goes when sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposeTarget {
    PullRequestComment,
    LineComment {
        path: String,
        line: u64,
        side: DiffSide,
        excerpt: String,
    },
    Review {
        event: ReviewEvent,
    },
}

/// Something the user asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    NextPane,
    PrevPane,
    Escape,
    Up,
    Down,
    Top,
    Bottom,
    PageUp,
    PageDown,
    Open,
    Refresh,
    ToggleTab,
    Comment,
    Review,
    Pick(ReviewEvent),
    Type(char),
    Newline,
    Backspace,
    Submit,
    Cancel,
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

/// A request that has been sent and not yet answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InFlight {
    PullRequests,
    Files(u64),
    Conversation(u64),
    Comment(u64),
    Review(u64),
}

/// The last outcome worth telling the user about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    pub error: bool,
}

/// The sizes the UI drew with last frame, so scrolling can be exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    pub conversation_width: usize,
    pub conversation_height: usize,
    pub diff_width: usize,
    pub diff_height: usize,
}

impl Default for Viewport {
    fn default() -> Viewport {
        Viewport {
            conversation_width: 80,
            conversation_height: 20,
            diff_width: 80,
            diff_height: 20,
        }
    }
}

/// One row of the diff pane: a patch line, or one wrapped line of a
/// comment shown under the patch line it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffRow {
    Line(DiffLine),
    Comment {
        /// `author · date` (or `pending`) on a comment's first row only.
        header: Option<String>,
        text: String,
        pending: bool,
    },
}

/// One pre-wrapped row of the conversation pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvRow {
    pub text: String,
    pub kind: ConvRowKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvRowKind {
    Title,
    Meta,
    Body,
    Heading,
    Author,
    Thread,
}

/// Rows moved by `PageUp`/`PageDown`.
const PAGE_LINES: usize = 20;
/// Rows kept visible above and below the diff cursor.
const CURSOR_MARGIN: usize = 3;

pub struct App {
    pub repo: RepoSlug,
    pub focus: Focus,
    pub tab: Tab,
    pub mode: Mode,
    pub prs: Vec<PullRequest>,
    pub pr_index: usize,
    /// The PR the user most recently opened; responses for others are dropped.
    pub opened: Option<u64>,
    pub files: Vec<ChangedFile>,
    pub file_index: usize,
    /// The PR whose files are currently shown.
    pub loaded_pr: Option<u64>,
    pub conversation: Option<Conversation>,
    pub conversation_for: Option<u64>,
    pub conversation_scroll: usize,
    pub diff_rows: Vec<DiffRow>,
    pub diff_cursor: usize,
    pub diff_offset: usize,
    pub pending_comments: Vec<NewLineComment>,
    pub in_flight: Vec<InFlight>,
    pub notice: Option<Notice>,
    pub viewport: Viewport,
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
        Move::PageUp => index.saturating_sub(PAGE_LINES),
        Move::PageDown => (index + PAGE_LINES).min(last),
    }
}

impl App {
    pub fn new(repo: RepoSlug) -> App {
        App {
            repo,
            focus: Focus::PullRequests,
            tab: Tab::Changes,
            mode: Mode::Browse,
            prs: Vec::new(),
            pr_index: 0,
            opened: None,
            files: Vec::new(),
            file_index: 0,
            loaded_pr: None,
            conversation: None,
            conversation_for: None,
            conversation_scroll: 0,
            diff_rows: Vec::new(),
            diff_cursor: 0,
            diff_offset: 0,
            pending_comments: Vec::new(),
            in_flight: Vec::new(),
            notice: None,
            viewport: Viewport::default(),
            should_quit: false,
        }
    }

    pub fn selected_pr(&self) -> Option<&PullRequest> {
        self.prs.get(self.pr_index)
    }

    pub fn selected_file(&self) -> Option<&ChangedFile> {
        self.files.get(self.file_index)
    }

    /// The PR the user opened, if it is still in the list.
    pub fn opened_pr(&self) -> Option<&PullRequest> {
        self.opened.and_then(|number| self.pr_by_number(number))
    }

    pub fn is_loading(&self, what: InFlight) -> bool {
        self.in_flight.contains(&what)
    }

    /// What the status bar says while work is in flight.
    pub fn busy_message(&self) -> Option<String> {
        self.in_flight.first().map(|item| match item {
            InFlight::PullRequests => "Loading pull requests…".to_string(),
            InFlight::Files(n) | InFlight::Conversation(n) => format!("Loading #{n}…"),
            InFlight::Comment(_) => "Posting comment…".to_string(),
            InFlight::Review(_) => "Submitting review…".to_string(),
        })
    }

    /// Records the sizes the UI just drew with and re-clamps scrolling.
    pub fn set_viewport(&mut self, viewport: Viewport) {
        if self.viewport == viewport {
            return;
        }
        self.viewport = viewport;
        self.rebuild_diff_rows();
        self.conversation_scroll = self.conversation_scroll.min(self.max_conversation_scroll());
    }

    pub fn conversation_row_count(&self) -> usize {
        match (&self.conversation, self.conversation_for) {
            (Some(conversation), Some(number)) => conversation_rows(
                number,
                self.pr_by_number(number),
                conversation,
                self.viewport.conversation_width,
            )
            .len(),
            _ => 0,
        }
    }

    /// Applies a user action; returns the requests to send.
    pub fn dispatch(&mut self, action: Action) -> Vec<Request> {
        self.notice = None;
        match self.mode {
            Mode::Browse => self.browse(action),
            Mode::PickReviewEvent | Mode::Compose(_) => Vec::new(),
        }
    }

    /// Absorbs a worker response; returns follow-up requests to send.
    pub fn apply(&mut self, response: Response) -> Vec<Request> {
        match response {
            Response::PullRequests(result) => {
                self.settle(InFlight::PullRequests);
                match result {
                    Ok(prs) => {
                        let previous = self.selected_pr().map(|pr| pr.number);
                        self.prs = prs;
                        self.pr_index = previous
                            .and_then(|number| self.prs.iter().position(|pr| pr.number == number))
                            .unwrap_or(0);
                    }
                    Err(message) => self.fail(message),
                }
            }
            Response::Files { number, result } => {
                self.settle(InFlight::Files(number));
                if self.opened != Some(number) {
                    return Vec::new();
                }
                match result {
                    Ok(files) => {
                        self.files = files;
                        self.loaded_pr = Some(number);
                        self.file_index = 0;
                        self.diff_cursor = 0;
                        self.diff_offset = 0;
                        self.rebuild_diff_rows();
                    }
                    Err(message) => self.fail(message),
                }
            }
            Response::Conversation { number, result } => {
                self.settle(InFlight::Conversation(number));
                if self.opened != Some(number) {
                    return Vec::new();
                }
                match result {
                    Ok(conversation) => {
                        self.conversation = Some(conversation);
                        self.conversation_for = Some(number);
                        self.conversation_scroll = 0;
                        self.rebuild_diff_rows();
                    }
                    Err(message) => self.fail(message),
                }
            }
            Response::CommentPosted { number, result } => {
                self.settle(InFlight::Comment(number));
                if let Err(message) = result {
                    self.fail(message);
                }
            }
            Response::ReviewSubmitted { number, result } => {
                self.settle(InFlight::Review(number));
                if let Err(message) = result {
                    self.fail(message);
                }
            }
        }
        Vec::new()
    }

    // ---- browse-mode actions ------------------------------------------------

    fn browse(&mut self, action: Action) -> Vec<Request> {
        match action {
            Action::Quit => self.should_quit = true,
            Action::NextPane => self.cycle_focus(1),
            Action::PrevPane => self.cycle_focus(-1),
            Action::Escape => self.escape(),
            Action::Up => self.navigate(Move::Up),
            Action::Down => self.navigate(Move::Down),
            Action::Top => self.navigate(Move::Top),
            Action::Bottom => self.navigate(Move::Bottom),
            Action::PageUp => self.page(Move::PageUp),
            Action::PageDown => self.page(Move::PageDown),
            Action::Open => return self.open(),
            Action::Refresh => return self.refresh(),
            Action::ToggleTab => self.toggle_tab(),
            Action::Comment
            | Action::Review
            | Action::Pick(_)
            | Action::Type(_)
            | Action::Newline
            | Action::Backspace
            | Action::Submit
            | Action::Cancel => {}
        }
        Vec::new()
    }

    fn cycle_focus(&mut self, step: isize) {
        let panes = self.tab.panes();
        let index = panes.iter().position(|&f| f == self.focus).unwrap_or(0) as isize;
        let next = (index + step).rem_euclid(panes.len() as isize) as usize;
        self.focus = panes[next];
    }

    fn escape(&mut self) {
        self.focus = match self.focus {
            Focus::Diff => Focus::Files,
            Focus::Files | Focus::Conversation | Focus::PullRequests => Focus::PullRequests,
        };
    }

    fn toggle_tab(&mut self) {
        self.tab = self.tab.other();
        if self.focus != Focus::PullRequests {
            self.focus = self.tab.first_pane();
        }
    }

    fn open(&mut self) -> Vec<Request> {
        match self.focus {
            Focus::PullRequests => {
                let Some(number) = self.selected_pr().map(|pr| pr.number) else {
                    return Vec::new();
                };
                self.opened = Some(number);
                self.focus = self.tab.first_pane();
                // Loads for other PRs would be dropped on arrival; stop
                // reporting them as busy.
                self.in_flight.retain(|item| match *item {
                    InFlight::Files(n) | InFlight::Conversation(n) => n == number,
                    _ => true,
                });
                let mut requests = Vec::new();
                if self.loaded_pr != Some(number) && !self.is_loading(InFlight::Files(number)) {
                    requests.push(self.track(Request::LoadFiles { number }));
                }
                if self.conversation_for != Some(number)
                    && !self.is_loading(InFlight::Conversation(number))
                {
                    requests.push(self.track(Request::LoadConversation { number }));
                }
                requests
            }
            Focus::Files => {
                self.focus = Focus::Diff;
                Vec::new()
            }
            Focus::Diff | Focus::Conversation => Vec::new(),
        }
    }

    fn refresh(&mut self) -> Vec<Request> {
        if self.is_loading(InFlight::PullRequests) {
            return Vec::new();
        }
        vec![self.track(Request::LoadPullRequests)]
    }

    fn navigate(&mut self, mv: Move) {
        match self.focus {
            Focus::PullRequests => self.pr_index = moved(self.pr_index, self.prs.len(), mv),
            Focus::Files => {
                let next = moved(self.file_index, self.files.len(), mv);
                if next != self.file_index {
                    self.file_index = next;
                    self.diff_cursor = 0;
                    self.diff_offset = 0;
                    self.rebuild_diff_rows();
                }
            }
            Focus::Diff => {
                self.diff_cursor = moved(self.diff_cursor, self.diff_rows.len(), mv);
                self.follow_cursor();
            }
            Focus::Conversation => {
                let len = self.max_conversation_scroll() + 1;
                self.conversation_scroll = moved(self.conversation_scroll, len, mv);
            }
        }
    }

    fn page(&mut self, mv: Move) {
        if matches!(self.focus, Focus::Diff | Focus::Conversation) {
            self.navigate(mv);
        }
    }

    // ---- bookkeeping --------------------------------------------------------

    fn track(&mut self, request: Request) -> Request {
        let item = match &request {
            Request::LoadPullRequests => InFlight::PullRequests,
            Request::LoadFiles { number } => InFlight::Files(*number),
            Request::LoadConversation { number } => InFlight::Conversation(*number),
            Request::PostComment { number, .. } => InFlight::Comment(*number),
            Request::SubmitReview { number, .. } => InFlight::Review(*number),
        };
        if !self.in_flight.contains(&item) {
            self.in_flight.push(item);
        }
        request
    }

    fn settle(&mut self, item: InFlight) {
        self.in_flight.retain(|other| *other != item);
    }

    fn fail(&mut self, message: String) {
        self.notice = Some(Notice {
            text: message,
            error: true,
        });
    }

    fn pr_by_number(&self, number: u64) -> Option<&PullRequest> {
        self.prs.iter().find(|pr| pr.number == number)
    }

    fn max_conversation_scroll(&self) -> usize {
        self.conversation_row_count()
            .saturating_sub(self.viewport.conversation_height)
    }

    /// The line number, side, and text of the patch line under the cursor,
    /// when a comment can be attached to it.
    pub fn commentable_at_cursor(&self) -> Option<(u64, DiffSide, String)> {
        self.selected_file()?;
        match self.diff_rows.get(self.diff_cursor)? {
            DiffRow::Line(line) => {
                anchor_of(line).map(|(number, side)| (number, side, line.text.clone()))
            }
            DiffRow::Comment { .. } => None,
        }
    }

    /// Lays the selected file out as rows: each patch line, then the review
    /// threads anchored to it, then the pending comments on it.
    fn rebuild_diff_rows(&mut self) {
        let width = self.viewport.diff_width.saturating_sub(6).max(10);
        let mut rows = Vec::new();
        if let Some(file) = self.selected_file() {
            let path = file.filename.as_str();
            let threads: Vec<_> = self
                .conversation
                .iter()
                .flat_map(|conversation| conversation.threads.iter())
                .filter(|thread| !thread.outdated && thread.path == path)
                .collect();
            let pending: Vec<_> = self
                .pending_comments
                .iter()
                .filter(|comment| comment.path == path)
                .collect();
            for line in file.patch.as_deref().map(parse_patch).unwrap_or_default() {
                let anchor = anchor_of(&line);
                rows.push(DiffRow::Line(line));
                let Some((number, side)) = anchor else {
                    continue;
                };
                for thread in threads
                    .iter()
                    .filter(|thread| thread.line == Some(number) && thread.side == side)
                {
                    for comment in &thread.comments {
                        let header = format!("{} · {}", comment.author, short_date(&comment.created_at));
                        push_comment_rows(&mut rows, header, &comment.body, false, width);
                    }
                }
                for comment in pending
                    .iter()
                    .filter(|comment| comment.line == number && comment.side == side)
                {
                    push_comment_rows(&mut rows, "pending".into(), &comment.body, true, width);
                }
            }
        }
        self.diff_rows = rows;
        self.diff_cursor = self.diff_cursor.min(self.diff_rows.len().saturating_sub(1));
        self.follow_cursor();
    }

    /// Scrolls the diff so the cursor stays `CURSOR_MARGIN` rows from the edges.
    fn follow_cursor(&mut self) {
        let height = self.viewport.diff_height.max(1);
        let margin = CURSOR_MARGIN.min(height.saturating_sub(1) / 2);
        if self.diff_cursor < self.diff_offset + margin {
            self.diff_offset = self.diff_cursor.saturating_sub(margin);
        } else if self.diff_cursor + margin + 1 > self.diff_offset + height {
            self.diff_offset = (self.diff_cursor + margin + 1).saturating_sub(height);
        }
        let max_offset = self.diff_rows.len().saturating_sub(height);
        self.diff_offset = self.diff_offset.min(max_offset);
    }
}

/// Where a comment on this patch line would attach: added and context lines
/// belong to the new file, removed lines to the old one.
fn anchor_of(line: &DiffLine) -> Option<(u64, DiffSide)> {
    match line.kind {
        DiffLineKind::Added | DiffLineKind::Context => {
            line.new_line.map(|number| (number, DiffSide::Right))
        }
        DiffLineKind::Removed => line.old_line.map(|number| (number, DiffSide::Left)),
        DiffLineKind::Hunk => None,
    }
}

fn push_comment_rows(rows: &mut Vec<DiffRow>, header: String, body: &str, pending: bool, width: usize) {
    let mut header = Some(header);
    for text in wrap(body, width) {
        rows.push(DiffRow::Comment {
            header: header.take(),
            text,
            pending,
        });
    }
}

// ---- conversation rendering helpers -----------------------------------------

/// Greedy word wrap of each logical line to `width` columns. Words longer
/// than `width` are split. Leading indentation is not preserved.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rows = Vec::new();
    for line in text.split('\n') {
        let mut current = String::new();
        let mut current_len = 0;
        for word in line.split_whitespace() {
            let word_len = word.chars().count();
            if current_len > 0 && current_len + 1 + word_len <= width {
                current.push(' ');
                current.push_str(word);
                current_len += 1 + word_len;
                continue;
            }
            if current_len > 0 {
                rows.push(std::mem::take(&mut current));
                current_len = 0;
            }
            for c in word.chars() {
                if current_len == width {
                    rows.push(std::mem::take(&mut current));
                    current_len = 0;
                }
                current.push(c);
                current_len += 1;
            }
        }
        rows.push(current);
    }
    rows
}

/// Lays the conversation pane out as rows, wrapped to `width`.
pub fn conversation_rows(
    number: u64,
    pr: Option<&PullRequest>,
    conversation: &Conversation,
    width: usize,
) -> Vec<ConvRow> {
    let row = |text: String, kind: ConvRowKind| ConvRow { text, kind };
    let blank = || ConvRow {
        text: String::new(),
        kind: ConvRowKind::Body,
    };
    let mut rows = Vec::new();
    match pr {
        Some(pr) => {
            rows.push(row(format!("#{number} {}", pr.title), ConvRowKind::Title));
            rows.push(row(
                format!("{} wants to merge {} into {}", pr.author, pr.head_ref, pr.base_ref),
                ConvRowKind::Meta,
            ));
        }
        None => rows.push(row(format!("#{number}"), ConvRowKind::Title)),
    }
    rows.push(blank());
    if conversation.body.trim().is_empty() {
        rows.push(row("No description.".into(), ConvRowKind::Body));
    } else {
        rows.extend(wrap(&conversation.body, width).into_iter().map(|t| row(t, ConvRowKind::Body)));
    }
    rows.push(blank());
    rows.push(row(format!("Comments ({})", conversation.comments.len()), ConvRowKind::Heading));
    for comment in &conversation.comments {
        let mut header = format!("{} · {}", comment.author, short_date(&comment.created_at));
        if let Some(state) = comment.review_state {
            header.push_str(" · ");
            header.push_str(review_label(state));
        }
        rows.push(row(header, ConvRowKind::Author));
        rows.extend(wrap(&comment.body, width).into_iter().map(|t| row(t, ConvRowKind::Body)));
        rows.push(blank());
    }
    let detached: Vec<_> = conversation
        .threads
        .iter()
        .filter(|thread| thread.outdated || thread.line.is_none())
        .collect();
    if !detached.is_empty() {
        rows.push(row(
            format!("Threads not on the current diff ({})", detached.len()),
            ConvRowKind::Heading,
        ));
        for thread in detached {
            rows.push(row(format!("{} (outdated)", thread.path), ConvRowKind::Thread));
            for comment in &thread.comments {
                rows.push(row(
                    format!("{} · {}", comment.author, short_date(&comment.created_at)),
                    ConvRowKind::Author,
                ));
                rows.extend(wrap(&comment.body, width).into_iter().map(|t| row(t, ConvRowKind::Body)));
            }
            rows.push(blank());
        }
    }
    rows
}

fn short_date(timestamp: &str) -> &str {
    timestamp.get(..10).unwrap_or(timestamp)
}

fn review_label(state: ReviewState) -> &'static str {
    match state {
        ReviewState::Approved => "approved",
        ReviewState::ChangesRequested => "changes requested",
        ReviewState::Commented => "commented",
        ReviewState::Dismissed => "dismissed",
        ReviewState::Pending => "pending",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::DiffLineKind;
    use crate::github::{ChecksState, Comment, FileStatus, ReviewDecision, ReviewState, Thread};

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

    fn two_files() -> Vec<ChangedFile> {
        vec![
            file("a.rs", Some("@@ -10,2 +20,2 @@\n a\n-b\n+c")),
            file("b.rs", Some("+z")),
        ]
    }

    fn comment(author: &str, date: &str, body: &str, state: Option<ReviewState>) -> Comment {
        Comment {
            author: author.into(),
            created_at: format!("{date}T00:00:00Z"),
            body: body.into(),
            review_state: state,
        }
    }

    fn thread(path: &str, line: Option<u64>, side: DiffSide, outdated: bool) -> Thread {
        Thread {
            path: path.into(),
            line,
            side,
            outdated,
            resolved: false,
            comments: vec![comment("ann", "2026-01-02", "why?", None)],
        }
    }

    fn line_row(row: &DiffRow) -> &DiffLine {
        match row {
            DiffRow::Line(line) => line,
            other => panic!("expected a diff line, got {other:?}"),
        }
    }

    /// PRs #1 and #2 loaded, #2 selected and opened (files and conversation requested).
    fn app_with_pr2_opened() -> App {
        let mut app = App::new(repo());
        app.apply(Response::PullRequests(Ok(vec![pr(1), pr(2)])));
        app.dispatch(Action::Down);
        assert_eq!(
            app.dispatch(Action::Open),
            vec![
                Request::LoadFiles { number: 2 },
                Request::LoadConversation { number: 2 }
            ]
        );
        app
    }

    /// `app_with_pr2_opened` with files and an empty conversation applied.
    fn loaded_app() -> App {
        let mut app = app_with_pr2_opened();
        app.apply(Response::Files { number: 2, result: Ok(two_files()) });
        app.apply(Response::Conversation { number: 2, result: Ok(Conversation::default()) });
        app
    }

    // ---- loading and selection ----

    #[test]
    fn starts_idle_and_refresh_starts_loading() {
        let mut app = App::new(repo());
        assert_eq!(app.focus, Focus::PullRequests);
        assert_eq!(app.tab, Tab::Changes);
        assert!(app.in_flight.is_empty());
        assert!(!app.should_quit);
        assert_eq!(app.dispatch(Action::Refresh), vec![Request::LoadPullRequests]);
        assert!(app.is_loading(InFlight::PullRequests));
        assert_eq!(app.dispatch(Action::Refresh), vec![]);
    }

    #[test]
    fn pull_requests_response_populates_list_and_clears_loading() {
        let mut app = App::new(repo());
        app.dispatch(Action::Refresh);
        app.apply(Response::PullRequests(Ok(vec![pr(1), pr(2)])));
        assert_eq!(app.prs.len(), 2);
        assert_eq!(app.pr_index, 0);
        assert!(!app.is_loading(InFlight::PullRequests));
        assert_eq!(app.notice, None);
    }

    #[test]
    fn pull_requests_error_sets_error_notice() {
        let mut app = App::new(repo());
        app.dispatch(Action::Refresh);
        app.apply(Response::PullRequests(Err("boom".into())));
        assert_eq!(app.notice, Some(Notice { text: "boom".into(), error: true }));
        assert!(!app.is_loading(InFlight::PullRequests));
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
    fn open_requests_files_and_conversation() {
        let app = app_with_pr2_opened();
        assert_eq!(app.opened, Some(2));
        assert_eq!(app.focus, Focus::Files);
        assert!(app.is_loading(InFlight::Files(2)));
        assert!(app.is_loading(InFlight::Conversation(2)));
    }

    #[test]
    fn open_with_conversation_tab_focuses_conversation() {
        let mut app = App::new(repo());
        app.apply(Response::PullRequests(Ok(vec![pr(1)])));
        app.dispatch(Action::ToggleTab);
        assert_eq!(app.focus, Focus::PullRequests);
        app.dispatch(Action::Open);
        assert_eq!(app.focus, Focus::Conversation);
    }

    #[test]
    fn open_on_empty_list_is_noop() {
        let mut app = App::new(repo());
        app.apply(Response::PullRequests(Ok(vec![])));
        assert_eq!(app.dispatch(Action::Open), vec![]);
        assert_eq!(app.focus, Focus::PullRequests);
        app.dispatch(Action::Down);
        app.dispatch(Action::Bottom);
        assert_eq!(app.pr_index, 0);
    }

    #[test]
    fn open_skips_already_loaded_parts() {
        let mut app = loaded_app();
        app.dispatch(Action::PrevPane);
        assert_eq!(app.focus, Focus::PullRequests);
        assert_eq!(app.dispatch(Action::Open), vec![]);
        assert_eq!(app.focus, Focus::Files);
    }

    #[test]
    fn open_in_files_focuses_diff() {
        let mut app = loaded_app();
        assert_eq!(app.dispatch(Action::Open), vec![]);
        assert_eq!(app.focus, Focus::Diff);
    }

    // ---- responses ----

    #[test]
    fn files_response_sets_files_and_diff_rows() {
        let app = loaded_app();
        assert_eq!(app.files.len(), 2);
        assert_eq!(app.file_index, 0);
        assert_eq!(app.loaded_pr, Some(2));
        assert_eq!(app.diff_rows.len(), 4);
        assert_eq!(line_row(&app.diff_rows[2]).kind, DiffLineKind::Removed);
        assert!(!app.is_loading(InFlight::Files(2)));
    }

    #[test]
    fn files_error_sets_error_notice() {
        let mut app = app_with_pr2_opened();
        app.apply(Response::Files { number: 2, result: Err("nope".into()) });
        assert_eq!(app.notice, Some(Notice { text: "nope".into(), error: true }));
        assert!(!app.is_loading(InFlight::Files(2)));
    }

    #[test]
    fn stale_files_response_is_ignored() {
        let mut app = app_with_pr2_opened();
        app.apply(Response::Files { number: 1, result: Ok(two_files()) });
        assert!(app.files.is_empty());
        assert_eq!(app.loaded_pr, None);
    }

    #[test]
    fn stale_conversation_is_dropped() {
        let mut app = app_with_pr2_opened();
        app.apply(Response::Conversation { number: 1, result: Ok(Conversation::default()) });
        assert_eq!(app.conversation, None);
        assert_eq!(app.conversation_for, None);
        assert!(app.is_loading(InFlight::Conversation(2)));
    }

    #[test]
    fn conversation_response_is_stored() {
        let app = loaded_app();
        assert_eq!(app.conversation_for, Some(2));
        assert!(app.conversation.is_some());
        assert!(!app.is_loading(InFlight::Conversation(2)));
    }

    #[test]
    fn files_with_no_patch_give_no_rows() {
        let mut app = app_with_pr2_opened();
        app.apply(Response::Files { number: 2, result: Ok(vec![file("bin.png", None)]) });
        assert!(app.diff_rows.is_empty());
        assert_eq!(app.loaded_pr, Some(2));
    }

    #[test]
    fn reopening_loaded_pr_drops_other_pending_loads() {
        let mut app = loaded_app();
        app.dispatch(Action::PrevPane);
        app.dispatch(Action::Up);
        assert_eq!(
            app.dispatch(Action::Open),
            vec![Request::LoadFiles { number: 1 }, Request::LoadConversation { number: 1 }]
        );
        app.dispatch(Action::PrevPane);
        app.dispatch(Action::Down);
        assert_eq!(app.dispatch(Action::Open), vec![]);
        assert_eq!(app.opened, Some(2));
        assert!(!app.is_loading(InFlight::Files(1)));
        app.apply(Response::Files { number: 1, result: Ok(vec![file("other.rs", Some("+o"))]) });
        assert_eq!(app.loaded_pr, Some(2));
        assert_eq!(app.files, two_files());
    }

    #[test]
    fn reopening_loaded_pr_keeps_a_pull_request_reload() {
        let mut app = loaded_app();
        app.dispatch(Action::PrevPane);
        app.dispatch(Action::Refresh);
        app.dispatch(Action::Open);
        assert!(app.is_loading(InFlight::PullRequests));
    }

    #[test]
    fn refresh_keeps_selection_by_number() {
        let mut app = App::new(repo());
        app.apply(Response::PullRequests(Ok(vec![pr(1), pr(2)])));
        app.dispatch(Action::Down);
        app.dispatch(Action::Refresh);
        app.apply(Response::PullRequests(Ok(vec![pr(3), pr(2)])));
        assert_eq!(app.pr_index, 1);
        assert_eq!(app.selected_pr().map(|p| p.number), Some(2));
    }

    // ---- navigation ----

    #[test]
    fn escape_walks_back_to_the_pr_list() {
        let mut app = loaded_app();
        app.dispatch(Action::NextPane);
        assert_eq!(app.focus, Focus::Diff);
        app.dispatch(Action::Escape);
        assert_eq!(app.focus, Focus::Files);
        app.dispatch(Action::Escape);
        assert_eq!(app.focus, Focus::PullRequests);
        app.dispatch(Action::Escape);
        assert_eq!(app.focus, Focus::PullRequests);
    }

    #[test]
    fn escape_from_conversation_returns_to_pr_list() {
        let mut app = loaded_app();
        app.dispatch(Action::ToggleTab);
        assert_eq!(app.focus, Focus::Conversation);
        app.dispatch(Action::Escape);
        assert_eq!(app.focus, Focus::PullRequests);
        assert_eq!(app.tab, Tab::Conversation);
    }

    #[test]
    fn toggle_tab_moves_focus_between_sides() {
        let mut app = loaded_app();
        app.dispatch(Action::ToggleTab);
        assert_eq!((app.tab, app.focus), (Tab::Conversation, Focus::Conversation));
        app.dispatch(Action::ToggleTab);
        assert_eq!((app.tab, app.focus), (Tab::Changes, Focus::Files));
        app.dispatch(Action::PrevPane);
        app.dispatch(Action::ToggleTab);
        assert_eq!((app.tab, app.focus), (Tab::Conversation, Focus::PullRequests));
    }

    #[test]
    fn panes_cycle_per_tab() {
        let mut app = App::new(repo());
        app.dispatch(Action::NextPane);
        assert_eq!(app.focus, Focus::Files);
        app.dispatch(Action::NextPane);
        assert_eq!(app.focus, Focus::Diff);
        app.dispatch(Action::NextPane);
        assert_eq!(app.focus, Focus::PullRequests);
        app.dispatch(Action::PrevPane);
        assert_eq!(app.focus, Focus::Diff);
        app.dispatch(Action::ToggleTab);
        app.dispatch(Action::NextPane);
        assert_eq!(app.focus, Focus::PullRequests);
        app.dispatch(Action::NextPane);
        assert_eq!(app.focus, Focus::Conversation);
    }

    #[test]
    fn moving_in_files_rebuilds_rows_and_resets_cursor() {
        let mut app = loaded_app();
        app.dispatch(Action::NextPane);
        app.dispatch(Action::Down);
        assert_eq!(app.diff_cursor, 1);
        app.dispatch(Action::PrevPane);
        app.dispatch(Action::Down);
        assert_eq!(app.file_index, 1);
        assert_eq!(app.diff_rows.len(), 1);
        assert_eq!(line_row(&app.diff_rows[0]).text, "+z");
        assert_eq!(app.diff_cursor, 0);
    }

    #[test]
    fn diff_cursor_clamps_and_pages() {
        let mut app = app_with_pr2_opened();
        let patch = "+l\n".repeat(30);
        app.apply(Response::Files { number: 2, result: Ok(vec![file("big.rs", Some(&patch))]) });
        app.dispatch(Action::NextPane);
        assert_eq!(app.focus, Focus::Diff);
        app.dispatch(Action::Up);
        assert_eq!(app.diff_cursor, 0);
        app.dispatch(Action::Down);
        assert_eq!(app.diff_cursor, 1);
        app.dispatch(Action::PageDown);
        assert_eq!(app.diff_cursor, 21);
        app.dispatch(Action::PageDown);
        assert_eq!(app.diff_cursor, 29);
        app.dispatch(Action::Top);
        assert_eq!(app.diff_cursor, 0);
        app.dispatch(Action::Bottom);
        assert_eq!(app.diff_cursor, 29);
        app.dispatch(Action::PageUp);
        assert_eq!(app.diff_cursor, 9);
    }

    #[test]
    fn diff_cursor_follows_with_margin() {
        let mut app = app_with_pr2_opened();
        let patch = "+l\n".repeat(100);
        app.apply(Response::Files { number: 2, result: Ok(vec![file("big.rs", Some(&patch))]) });
        app.set_viewport(Viewport { diff_height: 10, ..Viewport::default() });
        app.dispatch(Action::NextPane);
        for _ in 0..6 {
            app.dispatch(Action::Down);
        }
        assert_eq!((app.diff_cursor, app.diff_offset), (6, 0));
        app.dispatch(Action::Down);
        assert_eq!((app.diff_cursor, app.diff_offset), (7, 1));
        app.dispatch(Action::Bottom);
        assert_eq!((app.diff_cursor, app.diff_offset), (99, 90));
        app.dispatch(Action::Top);
        assert_eq!((app.diff_cursor, app.diff_offset), (0, 0));
    }

    #[test]
    fn page_keys_do_nothing_outside_diff() {
        let mut app = App::new(repo());
        app.apply(Response::PullRequests(Ok(vec![pr(1), pr(2)])));
        app.dispatch(Action::PageDown);
        assert_eq!(app.pr_index, 0);
    }

    #[test]
    fn conversation_scroll_clamps_to_rows_minus_height() {
        let mut app = app_with_pr2_opened();
        let conversation = Conversation { body: "line\n".repeat(30), ..Conversation::default() };
        app.apply(Response::Conversation { number: 2, result: Ok(conversation) });
        app.set_viewport(Viewport { conversation_height: 10, ..Viewport::default() });
        app.dispatch(Action::ToggleTab);
        assert_eq!(app.focus, Focus::Conversation);
        app.dispatch(Action::Up);
        assert_eq!(app.conversation_scroll, 0);
        app.dispatch(Action::Down);
        assert_eq!(app.conversation_scroll, 1);
        app.dispatch(Action::Bottom);
        assert_eq!(app.conversation_scroll, app.conversation_row_count() - 10);
        app.dispatch(Action::Down);
        assert_eq!(app.conversation_scroll, app.conversation_row_count() - 10);
        app.dispatch(Action::Top);
        assert_eq!(app.conversation_scroll, 0);
    }

    // ---- notices and busy state ----

    #[test]
    fn dispatch_clears_notice() {
        let mut app = App::new(repo());
        app.apply(Response::PullRequests(Err("boom".into())));
        assert!(app.notice.is_some());
        app.dispatch(Action::Down);
        assert_eq!(app.notice, None);
    }

    #[test]
    fn busy_message_names_first_in_flight_item() {
        let mut app = App::new(repo());
        assert_eq!(app.busy_message(), None);
        app.dispatch(Action::Refresh);
        assert_eq!(app.busy_message().as_deref(), Some("Loading pull requests…"));
        let app = app_with_pr2_opened();
        assert_eq!(app.busy_message().as_deref(), Some("Loading #2…"));
    }

    #[test]
    fn quit_sets_flag() {
        let mut app = App::new(repo());
        app.dispatch(Action::Quit);
        assert!(app.should_quit);
    }

    // ---- wrapping and conversation rows ----

    #[test]
    fn wrap_breaks_on_words() {
        assert_eq!(wrap("aa bb cc", 5), ["aa bb", "cc"]);
        assert_eq!(wrap("a\nb", 10), ["a", "b"]);
    }

    #[test]
    fn wrap_splits_words_longer_than_width() {
        assert_eq!(wrap("abcdefgh", 3), ["abc", "def", "gh"]);
        assert_eq!(wrap("", 10), [""]);
        assert_eq!(wrap("ab", 0), ["a", "b"]);
    }

    #[test]
    fn rows_for_empty_body() {
        let rows = conversation_rows(2, Some(&pr(2)), &Conversation::default(), 80);
        assert_eq!((rows[0].text.as_str(), rows[0].kind), ("#2 PR 2", ConvRowKind::Title));
        assert_eq!(rows[1].text, "ann wants to merge feat into main");
        assert!(rows.iter().any(|r| r.text == "No description." && r.kind == ConvRowKind::Body));
        assert!(rows.iter().any(|r| r.text == "Comments (0)" && r.kind == ConvRowKind::Heading));
    }

    #[test]
    fn rows_order_comments_and_mark_reviews() {
        let conversation = Conversation {
            body: "Hello".into(),
            comments: vec![
                comment("ann", "2026-01-02", "lgtm", Some(ReviewState::Approved)),
                comment("bob", "2026-01-03", "second", None),
            ],
            threads: vec![],
        };
        let rows = conversation_rows(2, Some(&pr(2)), &conversation, 80);
        let authors: Vec<&str> = rows
            .iter()
            .filter(|r| r.kind == ConvRowKind::Author)
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(authors, ["ann · 2026-01-02 · approved", "bob · 2026-01-03"]);
        assert!(rows.iter().any(|r| r.text == "lgtm" && r.kind == ConvRowKind::Body));
    }

    #[test]
    fn rows_list_outdated_threads() {
        let conversation = Conversation {
            body: String::new(),
            comments: vec![],
            threads: vec![
                thread("src/a.rs", Some(7), DiffSide::Right, true),
                thread("src/b.rs", None, DiffSide::Right, false),
                thread("src/c.rs", Some(3), DiffSide::Right, false),
            ],
        };
        let rows = conversation_rows(2, None, &conversation, 80);
        let threads: Vec<&str> = rows
            .iter()
            .filter(|r| r.kind == ConvRowKind::Thread)
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(threads, ["src/a.rs (outdated)", "src/b.rs (outdated)"]);
        assert_eq!(rows[0].text, "#2");
    }

    #[test]
    fn rows_wrap_long_body_lines() {
        let conversation = Conversation { body: "x".repeat(50), ..Conversation::default() };
        let rows = conversation_rows(2, None, &conversation, 20);
        assert!(rows.iter().filter(|r| r.kind == ConvRowKind::Body).all(|r| r.text.len() <= 20));
    }

    // ---- diff rows with comments (Task 5) ----

    fn app_with_threads(threads: Vec<Thread>) -> App {
        let mut app = app_with_pr2_opened();
        app.apply(Response::Files { number: 2, result: Ok(two_files()) });
        let conversation = Conversation { threads, ..Conversation::default() };
        app.apply(Response::Conversation { number: 2, result: Ok(conversation) });
        app
    }

    fn comment_row(row: &DiffRow) -> (Option<&str>, &str, bool) {
        match row {
            DiffRow::Comment { header, text, pending } => (header.as_deref(), text.as_str(), *pending),
            other => panic!("expected a comment row, got {other:?}"),
        }
    }

    #[test]
    fn threads_appear_under_their_line() {
        let app = app_with_threads(vec![thread("a.rs", Some(21), DiffSide::Right, false)]);
        assert_eq!(app.diff_rows.len(), 5);
        assert_eq!(line_row(&app.diff_rows[3]).text, "+c");
        assert_eq!(comment_row(&app.diff_rows[4]), (Some("ann · 2026-01-02"), "why?", false));
    }

    #[test]
    fn left_side_threads_match_old_line_numbers() {
        let app = app_with_threads(vec![thread("a.rs", Some(11), DiffSide::Left, false)]);
        assert_eq!(line_row(&app.diff_rows[2]).text, "-b");
        assert_eq!(comment_row(&app.diff_rows[3]).1, "why?");
        assert_eq!(line_row(&app.diff_rows[4]).text, "+c");
    }

    #[test]
    fn outdated_and_other_file_threads_are_skipped() {
        let app = app_with_threads(vec![
            thread("a.rs", Some(21), DiffSide::Right, true),
            thread("b.rs", Some(21), DiffSide::Right, false),
            thread("a.rs", Some(21), DiffSide::Left, false),
        ]);
        assert_eq!(app.diff_rows.len(), 4);
    }

    #[test]
    fn pending_comments_follow_existing_threads() {
        let mut app = app_with_threads(vec![thread("a.rs", Some(21), DiffSide::Right, false)]);
        app.pending_comments.push(NewLineComment {
            path: "a.rs".into(),
            line: 21,
            side: DiffSide::Right,
            body: "mine".into(),
        });
        app.rebuild_diff_rows();
        assert_eq!(app.diff_rows.len(), 6);
        assert_eq!(comment_row(&app.diff_rows[4]).0, Some("ann · 2026-01-02"));
        assert_eq!(comment_row(&app.diff_rows[5]), (Some("pending"), "mine", true));
    }

    #[test]
    fn long_comment_bodies_wrap_into_rows() {
        let mut thread = thread("a.rs", Some(21), DiffSide::Right, false);
        thread.comments[0].body = "word ".repeat(12).trim().to_string();
        let mut app = app_with_threads(vec![thread]);
        app.set_viewport(Viewport { diff_width: 30, ..Viewport::default() });
        let comment_rows: Vec<&DiffRow> = app
            .diff_rows
            .iter()
            .filter(|row| matches!(row, DiffRow::Comment { .. }))
            .collect();
        assert!(comment_rows.len() >= 3, "{comment_rows:?}");
        assert!(comment_row(comment_rows[0]).0.is_some());
        assert!(comment_rows[1..].iter().all(|row| comment_row(row).0.is_none()));
        assert!(comment_rows.iter().all(|row| comment_row(row).1.len() <= 24));
    }

    #[test]
    fn commentable_at_cursor_reports_side_and_line() {
        let mut app = app_with_threads(vec![thread("a.rs", Some(21), DiffSide::Right, false)]);
        app.dispatch(Action::NextPane);
        assert_eq!(app.focus, Focus::Diff);
        assert_eq!(app.commentable_at_cursor(), None);
        app.dispatch(Action::Down);
        assert_eq!(app.commentable_at_cursor(), Some((20, DiffSide::Right, " a".to_string())));
        app.dispatch(Action::Down);
        assert_eq!(app.commentable_at_cursor(), Some((11, DiffSide::Left, "-b".to_string())));
        app.dispatch(Action::Down);
        assert_eq!(app.commentable_at_cursor(), Some((21, DiffSide::Right, "+c".to_string())));
        app.dispatch(Action::Down);
        assert_eq!(app.commentable_at_cursor(), None);
    }
}
