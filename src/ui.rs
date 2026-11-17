//! Draws an [`App`] into a frame. Nothing here mutates state; the sizes it
//! drew with go back to the app as a [`Viewport`].

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::app::{
    App, Compose, ComposeTarget, ConvRow, ConvRowKind, DiffRow, Focus, Mode, Tab, Viewport,
    conversation_rows,
};
use crate::diff::DiffLineKind;
use crate::github::{ChecksState, FileStatus, PullRequest, ReviewDecision, ReviewEvent};

const TAB: &str = "    ";
const NO_PATCH: &str = "No diff available for this file";
const NO_PULL_REQUESTS: &str = "No open pull requests";
const NO_OPENED_PR: &str = "Open a pull request to read its conversation";

pub fn render(frame: &mut Frame, app: &App) -> Viewport {
    let [main, footer] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(2)]).areas(frame.area());
    let [left, right] =
        Layout::horizontal([Constraint::Min(32), Constraint::Percentage(65)]).areas(main);
    let [tab_bar, content] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(right);
    let files_height = (app.files.len() + 2).clamp(3, 12) as u16;
    let [files, diff] =
        Layout::vertical([Constraint::Length(files_height), Constraint::Min(0)]).areas(content);

    render_pull_requests(frame, app, left);
    render_tab_bar(frame, app, tab_bar);
    match app.tab {
        Tab::Changes => {
            render_files(frame, app, files);
            render_diff(frame, app, diff);
        }
        Tab::Conversation => render_conversation(frame, app, content),
    }
    render_status_bar(frame, app, footer);
    match &app.mode {
        Mode::Browse => {}
        Mode::PickReviewEvent => render_pick_overlay(frame, app),
        Mode::Compose(compose) => render_compose_overlay(frame, app, compose),
    }

    // Both sides are measured every frame so the app can clamp scrolling for
    // whichever tab the user switches to next.
    let conversation = Block::bordered().inner(content);
    let diff = Block::bordered().inner(diff);
    Viewport {
        conversation_width: conversation.width as usize,
        conversation_height: conversation.height as usize,
        diff_width: diff.width as usize,
        diff_height: diff.height as usize,
    }
}

fn pane(title: String, focused: bool) -> Block<'static> {
    let block = Block::bordered().title(format!(" {title} "));
    if focused {
        block.border_style(Style::new().fg(Color::Yellow))
    } else {
        block
    }
}

// ---- sidebar ----------------------------------------------------------------

fn render_pull_requests(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app.prs.iter().map(card).collect();
    let title = format!("Pull Requests · {}", app.repo);
    let list = List::new(items)
        .block(pane(title, app.focus == Focus::PullRequests))
        .highlight_style(Style::new().bg(Color::DarkGray));
    let mut state = ListState::default();
    if !app.prs.is_empty() {
        state.select(Some(app.pr_index));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

/// Three lines per pull request plus a blank separator. The status line is
/// kept under 30 columns so it fits the sidebar of an 80-column terminal.
fn card(pr: &PullRequest) -> ListItem<'static> {
    let title = Line::from(vec![
        Span::styled(format!("#{} ", pr.number), Style::new().fg(Color::Cyan).bold()),
        Span::styled(pr.title.clone(), Style::new().bold()),
    ]);
    let meta = Line::from(format!(" {} · {} → {}", pr.author, pr.head_ref, pr.base_ref)).dark_gray();
    let mut markers: Vec<Span> = Vec::new();
    match pr.checks {
        ChecksState::Success => markers.push("✓ ci".green()),
        ChecksState::Failure => markers.push("✗ ci".red()),
        ChecksState::Pending => markers.push("● ci".yellow()),
        ChecksState::None => {}
    }
    match pr.review_decision {
        ReviewDecision::Approved => markers.push("✓ approved".green()),
        ReviewDecision::ChangesRequested => markers.push("✗ changes".red()),
        ReviewDecision::ReviewRequired => markers.push("○ review".yellow()),
        ReviewDecision::None => {}
    }
    if pr.draft {
        markers.push("draft".dark_gray());
    }
    if pr.comment_count > 0 {
        markers.push(Span::from(format!("{} 💬", pr.comment_count)).dark_gray());
    }
    let mut status = vec![Span::raw(" ")];
    for (i, marker) in markers.into_iter().enumerate() {
        if i > 0 {
            status.push(Span::raw("  "));
        }
        status.push(marker);
    }
    ListItem::new(vec![title, meta, Line::from(status), Line::default()])
}

// ---- right side -------------------------------------------------------------

fn render_tab_bar(frame: &mut Frame, app: &App, area: Rect) {
    let tab = |label: &str, active: bool| {
        let style = if active {
            Style::new().fg(Color::Yellow).bold()
        } else {
            Style::new().fg(Color::DarkGray)
        };
        Span::styled(format!(" {label} "), style)
    };
    let line = Line::from(vec![
        tab("Changes", app.tab == Tab::Changes),
        Span::raw("│"),
        tab("Conversation", app.tab == Tab::Conversation),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_files(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .files
        .iter()
        .map(|file| {
            let color = match file.status {
                FileStatus::Added => Color::Green,
                FileStatus::Removed => Color::Red,
                _ => Color::Yellow,
            };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{} ", file.status.letter()), Style::new().fg(color)),
                Span::raw(file.filename.clone()),
                Span::styled(format!("  +{}", file.additions), Style::new().fg(Color::Green)),
                Span::styled(format!(" -{}", file.deletions), Style::new().fg(Color::Red)),
            ]))
        })
        .collect();
    let title = format!("Files ({})", app.files.len());
    let list = List::new(items)
        .block(pane(title, app.focus == Focus::Files))
        .highlight_style(Style::new().reversed());
    let mut state = ListState::default();
    if !app.files.is_empty() {
        state.select(Some(app.file_index));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_diff(frame: &mut Frame, app: &App, area: Rect) {
    let title = match (app.loaded_pr, app.selected_file()) {
        (Some(number), Some(file)) => format!("#{number} {}", file.filename),
        _ => "Diff".to_string(),
    };
    let block = pane(title, app.focus == Focus::Diff);
    let height = block.inner(area).height as usize;
    let cursor_visible = app.focus == Focus::Diff;
    let lines: Vec<Line> = match app.selected_file() {
        Some(file) if file.patch.is_none() => vec![Line::from(NO_PATCH).dark_gray()],
        _ => app
            .diff_rows
            .iter()
            .enumerate()
            .skip(app.diff_offset)
            .take(height)
            .map(|(index, row)| {
                let line = diff_row(row);
                if cursor_visible && index == app.diff_cursor {
                    line.reversed()
                } else {
                    line
                }
            })
            .collect(),
    };
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn diff_row(row: &DiffRow) -> Line<'static> {
    match row {
        DiffRow::Line(line) => {
            let style = match line.kind {
                DiffLineKind::Added => Style::new().fg(Color::Green),
                DiffLineKind::Removed => Style::new().fg(Color::Red),
                DiffLineKind::Hunk => Style::new().fg(Color::Cyan),
                DiffLineKind::Context => Style::new(),
            };
            Line::styled(line.text.replace('\t', TAB), style)
        }
        DiffRow::Comment {
            header,
            text,
            pending,
        } => {
            let color = if pending.is_some() { Color::Yellow } else { Color::Magenta };
            let mut spans = vec![Span::raw(TAB)];
            if let Some(header) = header {
                spans.push(Span::styled(format!("{header}: "), Style::new().fg(color).bold()));
            }
            spans.push(Span::styled(text.clone(), Style::new().fg(color)));
            Line::from(spans)
        }
    }
}

fn render_conversation(frame: &mut Frame, app: &App, area: Rect) {
    let title = match app.opened {
        Some(number) => format!("Conversation · #{number}"),
        None => "Conversation".to_string(),
    };
    let block = pane(title, app.focus == Focus::Conversation);
    let inner = block.inner(area);
    let lines: Vec<Line> = match (&app.conversation, app.conversation_for) {
        (Some(conversation), Some(number)) => {
            conversation_rows(number, app.pr_by_number(number), conversation, inner.width as usize)
                .into_iter()
                .skip(app.conversation_scroll)
                .take(inner.height as usize)
                .map(conversation_line)
                .collect()
        }
        _ if app.opened.is_some() => vec![Line::from("Loading…").dark_gray()],
        _ => vec![Line::from(NO_OPENED_PR).dark_gray()],
    };
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn conversation_line(row: ConvRow) -> Line<'static> {
    let style = match row.kind {
        ConvRowKind::Title | ConvRowKind::Author => Style::new().bold(),
        ConvRowKind::Meta => Style::new().fg(Color::DarkGray),
        ConvRowKind::Body => Style::new(),
        ConvRowKind::Heading => Style::new().fg(Color::Cyan),
        ConvRowKind::Thread => Style::new().fg(Color::Yellow),
    };
    Line::styled(row.text, style)
}

// ---- overlays ---------------------------------------------------------------

/// A box of `width_pct` percent of the width and `height` rows, centered.
fn centered(area: Rect, width_pct: u16, height: Constraint) -> Rect {
    let [_, middle, _] =
        Layout::vertical([Constraint::Fill(1), height, Constraint::Fill(1)]).areas(area);
    let [_, center, _] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Percentage(width_pct),
        Constraint::Fill(1),
    ])
    .areas(middle);
    center
}

fn render_pick_overlay(frame: &mut Frame, app: &App) {
    let area = centered(frame.area(), 60, Constraint::Length(5));
    let number = app.opened.unwrap_or(0);
    let block = Block::bordered()
        .title(format!(" Review #{number} "))
        .border_style(Style::new().fg(Color::Yellow));
    let text = Line::from(vec![
        "(a) approve".green().bold(),
        Span::raw("  "),
        "(r) request changes".red().bold(),
        Span::raw("  "),
        "(c) comment".bold(),
        Span::raw("  "),
        "Esc cancel".dark_gray(),
    ]);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(text).wrap(Wrap { trim: true }).block(block),
        area,
    );
}

fn render_compose_overlay(frame: &mut Frame, app: &App, compose: &Compose) {
    let area = centered(frame.area(), 70, Constraint::Percentage(50));
    let number = app.opened.unwrap_or(0);
    let title = match &compose.target {
        ComposeTarget::PullRequestComment => format!("Comment on #{number}"),
        ComposeTarget::LineComment { path, line, .. } => format!("Comment on {path}:{line}"),
        ComposeTarget::Review { event } => match event {
            ReviewEvent::Approve => format!("Approve #{number}"),
            ReviewEvent::RequestChanges => format!("Request changes on #{number}"),
            ReviewEvent::Comment => format!("Comment review on #{number}"),
        },
    };
    let block = Block::bordered()
        .title(format!(" {title} "))
        .border_style(Style::new().fg(Color::Yellow));
    let inner = block.inner(area);
    let [text_area, hint_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);

    let mut lines: Vec<Line> = Vec::new();
    if let ComposeTarget::LineComment { excerpt, .. } = &compose.target {
        lines.push(Line::from(excerpt.replace('\t', TAB)).dark_gray());
    }
    // The cursor is part of the text so it wraps with it and is always on
    // the last row drawn.
    let width = text_area.width as usize;
    for logical in format!("{}▏", compose.text).split('\n') {
        for piece in hard_wrap(logical, width) {
            lines.push(Line::from(piece));
        }
    }
    // Keep the end of the text, where the cursor is, on screen.
    let visible = text_area.height as usize;
    let skip = lines.len().saturating_sub(visible);
    let lines: Vec<Line> = lines.into_iter().skip(skip).collect();

    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines), text_area);
    frame.render_widget(
        Paragraph::new(Line::from("Ctrl-s send · Esc cancel · Enter newline").dark_gray()),
        hint_area,
    );
}

/// Splits one logical line into rows of at most `width` characters, keeping
/// every character (unlike the conversation wrapper, which drops whitespace).
fn hard_wrap(line: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let chars: Vec<char> = line.chars().collect();
    if chars.is_empty() {
        return vec![String::new()];
    }
    chars.chunks(width).map(|chunk| chunk.iter().collect()).collect()
}

// ---- status bar -------------------------------------------------------------

fn render_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let [row_one, row_two] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);
    // An error is the one thing the status bar exists to show; give it the
    // whole row so the status code survives on an 80-column terminal.
    if let Some(notice) = &app.notice
        && notice.error
    {
        let line = Line::from(format!(" Error: {}", notice.text)).red();
        frame.render_widget(Paragraph::new(line), row_one);
    } else {
        let context = Line::from(context_text(app));
        let status = if let Some(busy) = app.busy_message() {
            Line::from(busy).yellow()
        } else if let Some(notice) = &app.notice {
            Line::from(notice.text.clone()).green()
        } else if app.prs.is_empty() {
            Line::from(NO_PULL_REQUESTS).dark_gray()
        } else {
            Line::default()
        };
        let context_width = context.width() as u16;
        let [left, right] =
            Layout::horizontal([Constraint::Length(context_width), Constraint::Min(0)]).areas(row_one);
        frame.render_widget(Paragraph::new(context), left);
        frame.render_widget(Paragraph::new(status).right_aligned(), right);
    }
    frame.render_widget(Paragraph::new(Line::from(hints(app)).dark_gray()), row_two);
}

/// Where the user is, and how many line comments wait to be sent.
fn context_text(app: &App) -> String {
    let mut text = match (&app.mode, app.focus, app.opened) {
        (Mode::Compose(_), _, _) => "Composing…".to_string(),
        (Mode::PickReviewEvent, _, _) => "Pick a verdict".to_string(),
        (Mode::Browse, Focus::PullRequests, _) | (Mode::Browse, _, None) => "Pull requests".to_string(),
        (Mode::Browse, Focus::Files, Some(number)) => format!("#{number} · files"),
        (Mode::Browse, Focus::Diff, Some(number)) => match (app.selected_file(), app.commentable_at_cursor()) {
            (Some(file), Some((line, _, _))) => format!("#{number} · in {}:{line}", file.filename),
            (Some(file), None) => format!("#{number} · in {}", file.filename),
            (None, _) => format!("#{number} · files"),
        },
        (Mode::Browse, Focus::Conversation, Some(number)) => format!("#{number} · conversation"),
    };
    let pending = app.pending_comments.len();
    if pending > 0 {
        match app.pending_pr {
            Some(other) if app.opened != Some(other) => {
                text.push_str(&format!(" · {pending} pending on #{other}"));
            }
            _ => text.push_str(&format!(" · {pending} pending")),
        }
    }
    format!(" {text}")
}

fn hints(app: &App) -> &'static str {
    match (&app.mode, app.focus) {
        (Mode::Compose(_), _) => " Ctrl-s send  Enter newline  Esc cancel",
        (Mode::PickReviewEvent, _) => " a approve  r request changes  c comment  Esc cancel",
        (Mode::Browse, Focus::PullRequests) => " j/k move  Enter open  v tab  c comment  R review  r reload  q quit",
        (Mode::Browse, Focus::Files) => " j/k move  Enter open file  v conversation  c comment  R review  Esc back  q quit",
        (Mode::Browse, Focus::Diff) => " j/k move  c comment  d/D delete pending  R review  v conversation  Esc back  q quit",
        (Mode::Browse, Focus::Conversation) => " j/k scroll  c comment  R review  v changes  Esc back  q quit",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Action, App, Response};
    use crate::github::{
        ChangedFile, ChecksState, Comment, Conversation, DiffSide, FileStatus, PullRequest,
        ReviewDecision, ReviewEvent, ReviewState, Thread,
    };
    use crate::repo::RepoSlug;
    use ratatui::{Terminal, backend::TestBackend};

    fn repo() -> RepoSlug {
        RepoSlug::parse("owner/name").unwrap()
    }

    fn pr(number: u64, title: &str) -> PullRequest {
        PullRequest {
            number,
            title: title.into(),
            author: "ann".into(),
            head_ref: "feat".into(),
            base_ref: "main".into(),
            head_sha: "sha".into(),
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

    fn draw(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                render(frame, app);
            })
            .unwrap();
        terminal.backend().to_string()
    }

    fn draw_viewport(app: &App, width: u16, height: u16) -> Viewport {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut viewport = None;
        terminal
            .draw(|frame| {
                viewport = Some(render(frame, app));
            })
            .unwrap();
        viewport.unwrap()
    }

    fn conversation_with(body: &str) -> Conversation {
        Conversation {
            body: body.into(),
            comments: vec![Comment {
                author: "ann".into(),
                created_at: "2026-01-02T00:00:00Z".into(),
                body: "lgtm".into(),
                review_state: Some(ReviewState::Approved),
            }],
            threads: vec![],
            omitted: 0,
        }
    }

    fn app_with_file(patch: Option<&str>) -> App {
        let mut app = App::new(repo());
        app.apply(Response::PullRequests(Ok(vec![pr(1, "A rather long pull request title")])));
        app.dispatch(Action::Open);
        app.apply(Response::Files { number: 1, result: Ok(vec![file("src/a.rs", patch)]) });
        app
    }

    #[test]
    fn renders_pr_titles_and_repo() {
        let mut app = App::new(repo());
        app.apply(Response::PullRequests(Ok(vec![pr(123, "Fix login")])));
        let text = draw(&app, 80, 24);
        assert!(text.contains("#123"), "{text}");
        assert!(text.contains("Fix login"), "{text}");
        assert!(text.contains("owner/name"), "{text}");
    }

    #[test]
    fn renders_empty_message() {
        let mut app = App::new(repo());
        app.apply(Response::PullRequests(Ok(vec![])));
        let text = draw(&app, 80, 24);
        assert!(text.contains("No open pull requests"), "{text}");
    }

    #[test]
    fn renders_diff_lines_and_file_counts() {
        let app = app_with_file(Some("@@ -1 +1 @@\n-x\n+y"));
        let text = draw(&app, 80, 24);
        assert!(text.contains("src/a.rs"), "{text}");
        assert!(text.contains("+1"), "{text}");
        assert!(text.contains("-1"), "{text}");
        assert!(text.contains("-x"), "{text}");
        assert!(text.contains("+y"), "{text}");
        assert!(text.contains("#1 src/a.rs"), "{text}");
    }

    #[test]
    fn renders_no_patch_placeholder() {
        let app = app_with_file(None);
        let text = draw(&app, 80, 24);
        assert!(text.contains("No diff available"), "{text}");
    }

    #[test]
    fn renders_in_tiny_terminal() {
        let app = app_with_file(Some("@@ -1 +1 @@\n-x\n+y"));
        let _ = draw(&app, 20, 5);
        let _ = draw(&app, 1, 1);
    }

    #[test]
    fn renders_error_status() {
        let mut app = App::new(repo());
        app.apply(Response::PullRequests(Err("boom".into())));
        let text = draw(&app, 80, 24);
        assert!(text.contains("Error: boom"), "{text}");
    }


    #[test]
    fn renders_long_error_with_status_code_at_80_columns() {
        let mut app = App::new(repo());
        app.apply(Response::PullRequests(Err(
            "pull requests: GitHub returned 401 Unauthorized; check your token".into(),
        )));
        let text = draw(&app, 80, 24);
        assert!(text.contains("401 Unauthorized; check your token"), "{text}");
    }

    // ---- Task 8 ----

    #[test]
    fn cards_show_status_markers() {
        let mut app = App::new(repo());
        let mut pr = pr(123, "Fix login");
        pr.checks = ChecksState::Success;
        pr.review_decision = ReviewDecision::Approved;
        pr.draft = true;
        pr.comment_count = 2;
        app.apply(Response::PullRequests(Ok(vec![pr])));
        let text = draw(&app, 100, 30);
        assert!(text.contains("✓ ci"), "{text}");
        assert!(text.contains("✓ approved"), "{text}");
        assert!(text.contains("draft"), "{text}");
        assert!(text.contains("2 💬"), "{text}");
        assert!(text.contains("ann · feat → main"), "{text}");
    }

    #[test]
    fn card_without_checks_or_reviews_has_no_markers() {
        let mut app = App::new(repo());
        app.apply(Response::PullRequests(Ok(vec![pr(1, "Plain")])));
        let text = draw(&app, 100, 30);
        for marker in ["✓", "✗", "○", "●", "draft", "💬"] {
            assert!(!text.contains(marker), "unexpected {marker} in {text}");
        }
    }

    #[test]
    fn tab_bar_marks_active_tab() {
        let app = App::new(repo());
        let text = draw(&app, 100, 30);
        assert!(text.contains("Changes"), "{text}");
        assert!(text.contains("Conversation"), "{text}");
    }

    #[test]
    fn conversation_tab_renders_body_and_comments() {
        let mut app = app_with_file(Some("+x"));
        app.apply(Response::Conversation { number: 1, result: Ok(conversation_with("Hello body")) });
        app.dispatch(Action::ToggleTab);
        let text = draw(&app, 100, 30);
        assert!(text.contains("#1 A rather long pull request title"), "{text}");
        assert!(text.contains("Hello body"), "{text}");
        assert!(text.contains("ann · 2026-01-02 · approved"), "{text}");
        assert!(text.contains("lgtm"), "{text}");
    }

    #[test]
    fn status_bar_shows_file_and_line_when_in_diff() {
        let mut app = app_with_file(Some("@@ -1 +1 @@\n-x\n+y"));
        app.dispatch(Action::NextPane);
        app.dispatch(Action::Down);
        let text = draw(&app, 100, 30);
        assert!(text.contains("#1 · in src/a.rs:1"), "{text}");
    }

    #[test]
    fn status_bar_counts_pending_comments() {
        let mut app = app_with_file(Some("@@ -1 +1 @@\n-x\n+y"));
        app.dispatch(Action::NextPane);
        app.dispatch(Action::Down);
        app.dispatch(Action::Comment);
        app.dispatch(Action::Type('m'));
        app.dispatch(Action::Submit);
        let text = draw(&app, 100, 30);
        assert!(text.contains("1 pending"), "{text}");
    }

    #[test]
    fn render_returns_viewport_sizes() {
        let mut app = app_with_file(Some("+x"));
        let viewport = draw_viewport(&app, 120, 40);
        assert!(viewport.diff_height > 25 && viewport.diff_height < 40, "{viewport:?}");
        assert!(viewport.diff_width > 60 && viewport.diff_width < 80, "{viewport:?}");
        app.dispatch(Action::ToggleTab);
        let viewport = draw_viewport(&app, 120, 40);
        assert!(viewport.conversation_height > 25 && viewport.conversation_height < 40, "{viewport:?}");
        assert!(viewport.conversation_width > 60 && viewport.conversation_width < 80, "{viewport:?}");
    }

    #[test]
    fn renders_conversation_in_tiny_terminal() {
        let mut app = app_with_file(Some("+x"));
        app.apply(Response::Conversation { number: 1, result: Ok(conversation_with(&"x".repeat(500))) });
        app.dispatch(Action::ToggleTab);
        let _ = draw(&app, 20, 5);
        let _ = draw(&app, 1, 1);
    }

    // ---- Task 9 ----

    fn app_in_diff_on_added_line() -> App {
        let mut app = app_with_file(Some("@@ -1 +1 @@\n-x\n+y"));
        app.dispatch(Action::NextPane);
        app.dispatch(Action::Down);
        app.dispatch(Action::Down);
        assert_eq!(app.commentable_at_cursor().map(|c| c.0), Some(1));
        app
    }

    #[test]
    fn diff_shows_inline_comment_rows() {
        let mut app = app_with_file(Some("@@ -1 +1 @@\n-x\n+y"));
        let conversation = Conversation {
            threads: vec![Thread {
                path: "src/a.rs".into(),
                line: Some(1),
                side: DiffSide::Right,
                outdated: false,
                resolved: false,
                comments: vec![Comment {
                    author: "ann".into(),
                    created_at: "2026-01-02T00:00:00Z".into(),
                    body: "why?".into(),
                    review_state: None,
                }],
            }],
            ..Conversation::default()
        };
        app.apply(Response::Conversation { number: 1, result: Ok(conversation) });
        let text = draw(&app, 100, 30);
        assert!(text.contains("ann · 2026-01-02"), "{text}");
        assert!(text.contains("why?"), "{text}");
        assert!(text.find("+y").unwrap() < text.find("why?").unwrap(), "{text}");
    }

    #[test]
    fn compose_overlay_shows_title_and_text() {
        let mut app = app_in_diff_on_added_line();
        app.dispatch(Action::Comment);
        for c in "hello".chars() {
            app.dispatch(Action::Type(c));
        }
        let text = draw(&app, 100, 30);
        assert!(text.contains("Comment on src/a.rs:1"), "{text}");
        assert!(text.contains("hello▏"), "{text}");
        assert!(text.contains("+y"), "{text}");
        assert!(text.contains("Ctrl-s send"), "{text}");
    }

    #[test]
    fn pick_overlay_lists_verdicts() {
        let mut app = app_with_file(Some("+x"));
        app.dispatch(Action::Review);
        let text = draw(&app, 100, 30);
        assert!(text.contains("Review #1"), "{text}");
        assert!(text.contains("(a) approve"), "{text}");
        assert!(text.contains("(r) request changes"), "{text}");
        assert!(text.contains("(c) comment"), "{text}");
    }

    #[test]
    fn review_compose_overlay_names_the_verdict() {
        let mut app = app_with_file(Some("+x"));
        app.dispatch(Action::Review);
        app.dispatch(Action::Pick(ReviewEvent::RequestChanges));
        let text = draw(&app, 100, 30);
        assert!(text.contains("Request changes on #1"), "{text}");
    }

    #[test]
    fn ui_expands_tabs_in_diff_lines() {
        let app = app_with_file(Some("+\tx"));
        let text = draw(&app, 100, 30);
        assert!(text.contains("+    x"), "{text}");
    }

    #[test]
    fn overlays_render_in_tiny_terminal() {
        let mut app = app_in_diff_on_added_line();
        app.dispatch(Action::Comment);
        let _ = draw(&app, 20, 5);
        let _ = draw(&app, 1, 1);
        app.dispatch(Action::Cancel);
        app.dispatch(Action::Review);
        let _ = draw(&app, 20, 5);
    }

    // ---- final review fixes ----

    #[test]
    fn compose_overlay_wraps_long_lines_so_the_cursor_stays_visible() {
        let mut app = app_in_diff_on_added_line();
        app.dispatch(Action::Comment);
        for _ in 0..150 {
            app.dispatch(Action::Type('x'));
        }
        let text = draw(&app, 100, 30);
        assert!(text.contains("▏"), "{text}");
    }

    #[test]
    fn diff_hints_mention_deleting_pending_comments() {
        let app = app_in_diff_on_added_line();
        let text = draw(&app, 120, 30);
        assert!(text.contains("d/D"), "{text}");
    }
}
