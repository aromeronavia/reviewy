mod app;
mod auth;
mod diff;
mod github;
mod keymap;
mod repo;
mod ui;
mod worker;

use std::env;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use anyhow::Result;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};

use app::{Action, App, Request, Response};
use github::http::HttpGitHub;
use repo::RepoSlug;

const USAGE: &str = "usage: reviewy [owner/name]

Browse a repository's open pull requests and their diffs in the terminal.
Without an argument the repository is read from the current directory's
`origin` remote. The GitHub token comes from GITHUB_TOKEN, GH_TOKEN, or
`gh auth token`, in that order.";

/// How long one loop iteration waits for a key before checking the worker.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    match start(args.first().map(String::as_str)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("reviewy: {err:#}");
            ExitCode::FAILURE
        }
    }
}

/// Resolves everything that can fail before the terminal is taken over,
/// then runs the UI and always hands the terminal back.
fn start(repo_arg: Option<&str>) -> Result<()> {
    let repo = match repo_arg {
        Some(spec) => RepoSlug::parse(spec)?,
        None => RepoSlug::detect_from_git()?,
    };
    let token = auth::resolve_token()?;
    let source = Arc::new(HttpGitHub::new(repo.clone(), token)?);
    let (requests, responses) = worker::spawn(source);
    let mut app = App::new(repo);
    for request in app.dispatch(Action::Refresh) {
        requests.send(request)?;
    }

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app, &requests, &responses);
    ratatui::restore();
    result
}

fn run(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    requests: &Sender<Request>,
    responses: &Receiver<Response>,
) -> Result<()> {
    while !app.should_quit {
        terminal.draw(|frame| ui::render(frame, app))?;
        if event::poll(POLL_INTERVAL)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && let Some(action) = keymap::action_for(key, &app.mode)
        {
            for request in app.dispatch(action) {
                requests.send(request)?;
            }
        }
        while let Ok(response) = responses.try_recv() {
            for request in app.apply(response) {
                requests.send(request)?;
            }
        }
    }
    Ok(())
}
