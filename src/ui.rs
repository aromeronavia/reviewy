//! Draws an [`App`] into a frame. Nothing here mutates state.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, List, ListItem, ListState, Paragraph},
};

use crate::app::{App, DiffRow, Focus};
use crate::diff::DiffLineKind;
use crate::github::FileStatus;

const KEY_HINTS: &str = " j/k move  h/l/Tab pane  Enter open  r refresh  q quit";
const NO_PATCH: &str = "No diff available for this file";
const NO_PULL_REQUESTS: &str = "No open pull requests";

pub fn render(frame: &mut Frame, app: &App) {
    let [main, footer] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());
    let [left, right] =
        Layout::horizontal([Constraint::Min(30), Constraint::Percentage(65)]).areas(main);
    let files_height = (app.files.len() + 2).clamp(3, 12) as u16;
    let [files, diff] =
        Layout::vertical([Constraint::Length(files_height), Constraint::Min(0)]).areas(right);

    render_pull_requests(frame, app, left);
    render_files(frame, app, files);
    render_diff(frame, app, diff);
    render_footer(frame, app, footer);
}

fn pane(title: String, focused: bool) -> Block<'static> {
    let block = Block::bordered().title(format!(" {title} "));
    if focused {
        block.border_style(Style::new().fg(Color::Yellow))
    } else {
        block
    }
}

fn render_pull_requests(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .prs
        .iter()
        .map(|pr| {
            let draft = if pr.draft { "[draft] " } else { "" };
            ListItem::new(Line::from(vec![
                Span::styled(format!("#{} ", pr.number), Style::new().fg(Color::Cyan)),
                Span::raw(format!("{draft}{}", pr.title)),
                Span::styled(format!("  @{}", pr.author), Style::new().fg(Color::DarkGray)),
            ]))
        })
        .collect();
    let title = format!("Pull Requests · {}", app.repo);
    let list = List::new(items)
        .block(pane(title, app.focus == Focus::PullRequests))
        .highlight_style(Style::new().reversed());
    let mut state = ListState::default();
    if !app.prs.is_empty() {
        state.select(Some(app.pr_index));
    }
    frame.render_stateful_widget(list, area, &mut state);
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
    let lines: Vec<Line> = match app.selected_file() {
        Some(file) if file.patch.is_none() => vec![Line::from(NO_PATCH).dark_gray()],
        _ => app
            .diff_rows
            .iter()
            .map(|row| {
                let DiffRow::Line(line) = row;
                diff_line(line)
            })
            .collect(),
    };
    let scroll = u16::try_from(app.diff_offset).unwrap_or(u16::MAX);
    let paragraph = Paragraph::new(lines)
        .block(pane(title, app.focus == Focus::Diff))
        .scroll((scroll, 0));
    frame.render_widget(paragraph, area);
}

fn diff_line(line: &crate::diff::DiffLine) -> Line<'_> {
    let style = match line.kind {
        DiffLineKind::Added => Style::new().fg(Color::Green),
        DiffLineKind::Removed => Style::new().fg(Color::Red),
        DiffLineKind::Hunk => Style::new().fg(Color::Cyan),
        DiffLineKind::Context => Style::new(),
    };
    Line::styled(line.text.as_str(), style)
}

fn render_footer(frame: &mut Frame, app: &App, area: Rect) {
    // An error is the one thing the footer exists to show; give it the whole
    // row so the status code survives on an 80-column terminal.
    if let Some(notice) = &app.notice
        && notice.error
    {
        let line = Line::from(format!(" Error: {}", notice.text)).red();
        frame.render_widget(Paragraph::new(line), area);
        return;
    }
    let status = if let Some(busy) = app.busy_message() {
        Line::from(busy).yellow()
    } else if let Some(notice) = &app.notice {
        Line::from(notice.text.clone()).green()
    } else if app.prs.is_empty() {
        Line::from(NO_PULL_REQUESTS).dark_gray()
    } else {
        Line::default()
    };
    let hints_width = KEY_HINTS.len() as u16;
    let [hints, right] =
        Layout::horizontal([Constraint::Length(hints_width), Constraint::Min(0)]).areas(area);
    frame.render_widget(Paragraph::new(Line::from(KEY_HINTS).dark_gray()), hints);
    frame.render_widget(Paragraph::new(status).right_aligned(), right);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Action, App, Response};
    use crate::github::{ChangedFile, ChecksState, FileStatus, PullRequest, ReviewDecision};
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
        terminal.draw(|frame| render(frame, app)).unwrap();
        terminal.backend().to_string()
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
}
