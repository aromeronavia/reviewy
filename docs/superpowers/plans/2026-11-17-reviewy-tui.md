# Reviewy TUI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Rust/Ratatui terminal app that lists one repository's open GitHub pull requests on the left and shows the selected PR's changed files and per-file diff on the right.

**Architecture:** Pure state machine (`App`) driven by `Action`s from the keymap and `Response`s from a single background worker thread that talks to the GitHub REST API through a `PullRequestSource` trait. The UI draws `App` each frame. No async runtime; threads talk over `std::sync::mpsc`.

**Tech Stack:** Rust 2021, ratatui 0.30 (crossterm via `ratatui::crossterm`), reqwest 0.13 blocking + json, serde + serde_json, anyhow.

**Spec:** `docs/superpowers/specs/2026-11-17-reviewy-design.md`

## Global Constraints

- Rust 2021 edition on the stable toolchain already installed (cargo 1.100).
- Dependencies limited to: `ratatui = "0.30"`, `reqwest = { version = "0.13", features = ["blocking", "json"] }`, `serde = { version = "1", features = ["derive"] }`, `serde_json = "1"`, `anyhow = "1"`.
- Crossterm types come only from `ratatui::crossterm`; never add `crossterm` to `Cargo.toml`.
- `app.rs` imports nothing from `ratatui` or `reqwest`.
- The token is never printed, logged, or put in an error message.
- Only `main.rs` touches the terminal; only `github/http.rs`, `auth.rs`, and `repo.rs` touch the network or subprocesses.
- Commit after every task with a conventional-commit message.

## Review Focus

1. A changed file whose `patch` is `null` (binary or huge): the diff pane shows `No diff available for this file` and nothing panics. Test in Task 4 (`apply` with `patch: None`) and Task 6 (render shows the message).
2. A repository with zero open PRs: footer shows `No open pull requests`, and `j`/`k`/`Enter` are no-ops. Test in Task 4 (`open_on_empty_list_is_noop`) and Task 6 (`renders_empty_message`).
3. Files for PR A arrive after the user already asked for PR B: A's files are dropped. Test in Task 4 (`stale_files_response_is_ignored`).
4. Terminal narrower than the layout wants (20x5): render must not panic. Test in Task 6 (`renders_in_tiny_terminal`).
5. `origin` is not a GitHub remote, or no token can be found: a one-line message on stderr before raw mode is ever entered, exit status 1. Test in Task 1 (`rejects_non_github_remote`); token path verified manually in Task 8.

---

### Task 1: Project scaffold and repository slug

**Files:**
- Create: `Cargo.toml`, `.gitignore`, `src/main.rs` (placeholder), `src/repo.rs`

**Interfaces:**
- Produces: `RepoSlug { pub owner: String, pub name: String }` with `RepoSlug::parse(spec: &str) -> anyhow::Result<RepoSlug>`, `RepoSlug::from_remote_url(url: &str) -> anyhow::Result<RepoSlug>`, `RepoSlug::detect_from_git() -> anyhow::Result<RepoSlug>`, and `impl Display` printing `owner/name`.

- [ ] **Step 1: Scaffold**

Run: `cd /Users/albertoromero/development/reviewy && git init -b main && cargo init --name reviewy && cargo add ratatui@0.30 anyhow@1 serde_json@1 && cargo add serde@1 --features derive && cargo add reqwest@0.13 --features blocking,json`
Expected: `Cargo.toml` lists exactly those five dependencies. Add `/target` to `.gitignore` if `cargo init` did not.

- [ ] **Step 2: Write the failing tests in `src/repo.rs`** (module `#[cfg(test)] mod tests`)

```rust
#[test] fn parses_owner_slash_name() { let r = RepoSlug::parse("ratatui/ratatui").unwrap(); assert_eq!((r.owner.as_str(), r.name.as_str()), ("ratatui", "ratatui")); }
#[test] fn rejects_missing_slash() { assert!(RepoSlug::parse("ratatui").is_err()); }
#[test] fn rejects_extra_segments() { assert!(RepoSlug::parse("a/b/c").is_err()); }
#[test] fn parses_ssh_remote() { assert_eq!(RepoSlug::from_remote_url("git@github.com:owner/name.git").unwrap().to_string(), "owner/name"); }
#[test] fn parses_https_remote_with_and_without_git_suffix() { for u in ["https://github.com/owner/name", "https://github.com/owner/name.git"] { assert_eq!(RepoSlug::from_remote_url(u).unwrap().to_string(), "owner/name"); } }
#[test] fn parses_ssh_url_scheme() { assert_eq!(RepoSlug::from_remote_url("ssh://git@github.com/owner/name.git").unwrap().to_string(), "owner/name"); }
#[test] fn rejects_non_github_remote() { let err = RepoSlug::from_remote_url("https://gitlab.com/owner/name.git").unwrap_err(); assert!(err.to_string().contains("GitHub")); }
```

- [ ] **Step 3: Run `cargo test repo::` and confirm it fails to compile** (no `RepoSlug`).

- [ ] **Step 4: Implement `src/repo.rs`**

`parse` splits on `/` and requires exactly two non-empty parts. `from_remote_url` strips a `git@github.com:`, `https://github.com/`, or `ssh://git@github.com/` prefix (error mentioning `GitHub` otherwise), strips a trailing `.git` and `/`, then calls `parse`. `detect_from_git` runs `git remote get-url origin` with `std::process::Command`, errors with `not a git repository or no origin remote` on failure, then calls `from_remote_url` on the trimmed stdout. Declare `mod repo;` in `main.rs`.

- [ ] **Step 5: Run `cargo test repo::`** — Expected: 7 passed.

- [ ] **Step 6: Commit** — `git add -A && git commit -m "feat: scaffold project and parse repository slugs"`

---

### Task 2: Diff line parsing

**Files:**
- Create: `src/diff.rs`

**Interfaces:**
- Produces: `enum DiffLineKind { Hunk, Added, Removed, Context }` (Copy, Eq), `struct DiffLine { pub kind: DiffLineKind, pub text: String }` (Clone, Eq), `pub fn parse_patch(patch: &str) -> Vec<DiffLine>`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn classifies_each_line_kind() {
    let lines = parse_patch("@@ -1,2 +1,2 @@\n ctx\n-old\n+new\n\\ No newline at end of file");
    let kinds: Vec<_> = lines.iter().map(|l| l.kind).collect();
    assert_eq!(kinds, [DiffLineKind::Hunk, DiffLineKind::Context, DiffLineKind::Removed, DiffLineKind::Added, DiffLineKind::Context]);
    assert_eq!(lines[2].text, "-old");
}
#[test] fn expands_tabs_to_four_spaces() { assert_eq!(parse_patch("+\tx")[0].text, "+    x"); }
#[test] fn empty_patch_gives_no_lines() { assert!(parse_patch("").is_empty()); }
```

- [ ] **Step 2: Run `cargo test diff::`** — Expected: compile failure.

- [ ] **Step 3: Implement `parse_patch`** — `patch.lines()` mapped by first character; `text` keeps the leading marker so columns line up. Declare `mod diff;` in `main.rs`.

- [ ] **Step 4: Run `cargo test diff::`** — Expected: 3 passed.

- [ ] **Step 5: Commit** — `git commit -am "feat: parse unified diff patches into typed lines"` (add the new file first).

---

### Task 3: GitHub models, source trait, and token resolution

**Files:**
- Create: `src/github/mod.rs`, `src/auth.rs`

**Interfaces:**
- Produces in `github/mod.rs`:
  - `struct PullRequest { pub number: u64, pub title: String, pub author: String, pub head_ref: String, pub base_ref: String, pub draft: bool }` (Clone, Eq, Debug)
  - `enum FileStatus { Added, Modified, Removed, Renamed, Other }` (Copy, Eq) with `FileStatus::from_api(s: &str) -> FileStatus` and `fn letter(self) -> char` (`A`, `M`, `D`, `R`, `?`)
  - `struct ChangedFile { pub filename: String, pub status: FileStatus, pub additions: u64, pub deletions: u64, pub patch: Option<String> }` (Clone, Eq, Debug)
  - `trait PullRequestSource: Send + Sync { fn list_open(&self) -> anyhow::Result<Vec<PullRequest>>; fn changed_files(&self, number: u64) -> anyhow::Result<Vec<ChangedFile>>; }`
  - `pub mod http;` is declared here but the file is written in Task 7; leave the line commented until then.
- Produces in `auth.rs`: `pub fn resolve_token() -> anyhow::Result<String>` and private `fn clean_token(raw: &str) -> Option<String>`.

- [ ] **Step 1: Write the failing tests**

In `github/mod.rs`:
```rust
#[test] fn maps_api_status_strings() { assert_eq!(FileStatus::from_api("added"), FileStatus::Added); assert_eq!(FileStatus::from_api("modified"), FileStatus::Modified); assert_eq!(FileStatus::from_api("removed"), FileStatus::Removed); assert_eq!(FileStatus::from_api("renamed"), FileStatus::Renamed); assert_eq!(FileStatus::from_api("copied"), FileStatus::Other); }
#[test] fn status_letters() { assert_eq!(FileStatus::Added.letter(), 'A'); assert_eq!(FileStatus::Modified.letter(), 'M'); assert_eq!(FileStatus::Removed.letter(), 'D'); assert_eq!(FileStatus::Renamed.letter(), 'R'); assert_eq!(FileStatus::Other.letter(), '?'); }
```
In `auth.rs`:
```rust
#[test] fn clean_token_trims_whitespace() { assert_eq!(clean_token("  gho_abc\n").as_deref(), Some("gho_abc")); }
#[test] fn clean_token_rejects_blank() { assert_eq!(clean_token(" \n"), None); }
```

- [ ] **Step 2: Run `cargo test`** — Expected: compile failure.

- [ ] **Step 3: Implement both files.** `resolve_token` checks `GITHUB_TOKEN`, then `GH_TOKEN`, then stdout of `gh auth token`, each through `clean_token`; the final error text is exactly `No GitHub token found. Set GITHUB_TOKEN or run \`gh auth login\`.` Declare `mod github; mod auth;` in `main.rs`.

- [ ] **Step 4: Run `cargo test`** — Expected: all previous plus 4 new pass.

- [ ] **Step 5: Commit** — `git add -A && git commit -m "feat: add GitHub models, source trait, and token resolution"`

---

### Task 4: Application state machine

**Files:**
- Create: `src/app.rs`

**Interfaces:**
- Consumes: `PullRequest`, `ChangedFile` (Task 3); `DiffLine`, `parse_patch` (Task 2); `RepoSlug` (Task 1).
- Produces:
  - `enum Focus { PullRequests, Files, Diff }` (Copy, Eq)
  - `enum Action { Quit, NextPane, PrevPane, Up, Down, Top, Bottom, PageUp, PageDown, Open, Refresh }` (Copy, Eq)
  - `enum Request { LoadPullRequests, LoadFiles { number: u64 } }` (Clone, Eq, Debug)
  - `enum Response { PullRequests(Result<Vec<PullRequest>, String>), Files { number: u64, result: Result<Vec<ChangedFile>, String> } }` (Debug)
  - `enum Status { Idle, Loading(String), Error(String) }` (Clone, Eq)
  - `pub const PAGE: usize = 20;`
  - `struct App { pub repo: RepoSlug, pub focus: Focus, pub prs: Vec<PullRequest>, pub pr_index: usize, pub files: Vec<ChangedFile>, pub file_index: usize, pub loaded_pr: Option<u64>, pub pending_pr: Option<u64>, pub diff: Vec<DiffLine>, pub diff_scroll: usize, pub status: Status, pub should_quit: bool }`
  - `App::new(repo: RepoSlug) -> App` (focus `PullRequests`, status `Loading("Loading pull requests…")`)
  - `App::selected_pr(&self) -> Option<&PullRequest>`, `App::selected_file(&self) -> Option<&ChangedFile>`
  - `App::dispatch(&mut self, action: Action) -> Option<Request>`
  - `App::apply(&mut self, response: Response)`

**Rules `dispatch` must follow:**
- `Quit` sets `should_quit`.
- `NextPane`/`PrevPane` cycle `PullRequests → Files → Diff` and wrap.
- `Up`/`Down`/`Top`/`Bottom` act on the focused pane. In `Files` any selection change re-parses `diff` from the selected file's `patch` (empty `Vec` when `None`) and resets `diff_scroll`. In `Diff` they move `diff_scroll`, clamped to `diff.len().saturating_sub(1)`.
- `PageUp`/`PageDown` move `diff_scroll` by `PAGE`, clamped, and only when focus is `Diff`.
- `Open` in `PullRequests`: when a PR is selected and `loaded_pr != Some(number)`, set `pending_pr`, status `Loading(format!("Loading #{number}…"))`, focus `Files`, return `Some(Request::LoadFiles { number })`. When already loaded, only move focus. `Open` in `Files` moves focus to `Diff`. Otherwise `None`.
- `Refresh` sets status `Loading("Loading pull requests…")` and returns `Some(Request::LoadPullRequests)`.

**Rules `apply` must follow:**
- `PullRequests(Ok(list))`: replace `prs`; keep `pr_index` on the previously selected PR number if it is still present, else `0`; status `Idle`.
- `PullRequests(Err(e))`: status `Error(e)`.
- `Files { number, Ok(files) }`: ignored unless `pending_pr == Some(number)`. Otherwise set `files`, `file_index = 0`, `loaded_pr = Some(number)`, `pending_pr = None`, re-parse `diff`, `diff_scroll = 0`, status `Idle`.
- `Files { Err(e), .. }`: `pending_pr = None`, status `Error(e)`.

- [ ] **Step 1: Write the failing tests** (helper `fn pr(n: u64) -> PullRequest` and `fn file(name: &str, patch: Option<&str>) -> ChangedFile` in the test module)

```rust
#[test] fn starts_loading_with_pr_pane_focused()
#[test] fn pull_requests_response_populates_list_and_clears_status()
#[test] fn down_and_up_clamp_within_pr_list()            // 2 PRs: Down, Down → 1; Up, Up → 0
#[test] fn open_requests_files_for_selected_pr()         // returns LoadFiles{number:2}, focus Files, status Loading("Loading #2…"), pending_pr Some(2)
#[test] fn open_on_empty_list_is_noop()                  // None, focus unchanged, no panic
#[test] fn files_response_sets_files_and_diff()          // first file's patch parsed, loaded_pr Some(2), status Idle
#[test] fn stale_files_response_is_ignored()             // pending 2, response for 1 → files still empty
#[test] fn files_with_no_patch_give_empty_diff()
#[test] fn moving_in_files_reparses_diff_and_resets_scroll()
#[test] fn diff_scroll_clamps_and_pages()                // 30-line diff: Down → 1; PageDown → 21; PageDown → 29; Bottom → 29; Top → 0
#[test] fn panes_cycle_and_wrap()
#[test] fn refresh_requests_reload_and_keeps_selection()  // select PR 2, refresh, apply list [3,2] → pr_index 1
#[test] fn quit_sets_flag()
```

- [ ] **Step 2: Run `cargo test app::`** — Expected: compile failure.

- [ ] **Step 3: Implement `src/app.rs`** per the rules above. Declare `mod app;` in `main.rs`.

- [ ] **Step 4: Run `cargo test app::`** — Expected: 13 passed.

- [ ] **Step 5: Commit** — `git add -A && git commit -m "feat: add application state machine"`

---

### Task 5: Keymap

**Files:**
- Create: `src/keymap.rs`

**Interfaces:**
- Consumes: `Action` (Task 4).
- Produces: `pub fn action_for(key: ratatui::crossterm::event::KeyEvent) -> Option<Action>`.

Bindings: `q`, `Ctrl-c` → `Quit`; `Tab`, `l` → `NextPane`; `BackTab`, `h` → `PrevPane`; `j`, `Down` → `Down`; `k`, `Up` → `Up`; `g`, `Home` → `Top`; `G`, `End` → `Bottom`; `Ctrl-d`, `PageDown` → `PageDown`; `Ctrl-u`, `PageUp` → `PageUp`; `Enter` → `Open`; `r` → `Refresh`; anything else → `None`.

- [ ] **Step 1: Write the failing tests** (helper `fn key(code: KeyCode) -> KeyEvent` and `fn ctrl(c: char) -> KeyEvent`)

```rust
#[test] fn maps_navigation_keys()   // j/Down→Down, k/Up→Up, g→Top, G→Bottom, Ctrl-d→PageDown, Ctrl-u→PageUp
#[test] fn maps_pane_and_action_keys() // Tab/l→NextPane, BackTab/h→PrevPane, Enter→Open, r→Refresh, q/Ctrl-c→Quit
#[test] fn unknown_key_is_none()    // 'x' → None
```

- [ ] **Step 2: Run `cargo test keymap::`** — Expected: compile failure.

- [ ] **Step 3: Implement** with one `match (key.code, key.modifiers.contains(KeyModifiers::CONTROL))`. Declare `mod keymap;` in `main.rs`.

- [ ] **Step 4: Run `cargo test keymap::`** — Expected: 3 passed.

- [ ] **Step 5: Commit** — `git add -A && git commit -m "feat: map key events to actions"`

---

### Task 6: UI rendering

**Files:**
- Create: `src/ui.rs`

**Interfaces:**
- Consumes: `App`, `Focus`, `Status` (Task 4); `DiffLineKind` (Task 2); `FileStatus` (Task 3).
- Produces: `pub fn render(frame: &mut ratatui::Frame, app: &App)`.

Layout per spec: vertical `[Min(0), Length(1)]`; main row horizontal `[Percentage(35), Percentage(65)]`; right column vertical `[Length(files_height), Min(0)]` where `files_height = (app.files.len() + 2).clamp(3, 12)`. Focused pane border is `Color::Yellow`, others default. PR item text: `#{number} {[draft] }{title}  @{author}`. File item text: `{letter} {filename}  +{additions} -{deletions}` with the letter green for `A`, red for `D`, yellow otherwise. Diff lines: `Added` green, `Removed` red, `Hunk` cyan, `Context` default; the `Paragraph` is scrolled by `app.diff_scroll`. Diff pane title: `#{number} {filename}` when a file is selected, else `Diff`. When a file is selected and its `patch` is `None`, show `No diff available for this file`. Footer left text: `j/k move  h/l/Tab pane  Enter open  r refresh  q quit`; footer right text: `Loading…` message, `Error: {e}` in red, `No open pull requests` when `prs` is empty and status is `Idle`, else empty. Lists use `ListState::default().with_selected(Some(index))` only when the list is non-empty, with `highlight_style(Style::new().reversed())`.

- [ ] **Step 1: Write the failing tests** (helper `fn text_of(terminal: &Terminal<TestBackend>) -> String` that joins every cell symbol row by row with newlines; helper `fn draw(app: &App, w: u16, h: u16) -> String`)

```rust
#[test] fn renders_pr_titles_and_repo()      // 80x24, one PR #123 "Fix login" → text contains "#123", "Fix login", "owner/name"
#[test] fn renders_empty_message()           // status Idle, no PRs → contains "No open pull requests"
#[test] fn renders_no_patch_placeholder()    // file with patch None selected → contains "No diff available"
#[test] fn renders_in_tiny_terminal()        // 20x5 with PRs and files → does not panic
#[test] fn renders_error_status()            // status Error("boom") → contains "Error: boom"
```

- [ ] **Step 2: Run `cargo test ui::`** — Expected: compile failure.

- [ ] **Step 3: Implement `src/ui.rs`** with private `render_pull_requests`, `render_files`, `render_diff`, `render_footer`, and `fn pane(title: String, focused: bool) -> Block<'static>`. Declare `mod ui;` in `main.rs`.

- [ ] **Step 4: Run `cargo test ui::`** — Expected: 5 passed.

- [ ] **Step 5: Commit** — `git add -A && git commit -m "feat: render pull requests, files, and diff panes"`

---

### Task 7: Background worker and HTTP GitHub client

**Files:**
- Create: `src/worker.rs`, `src/github/http.rs`
- Modify: `src/github/mod.rs` (uncomment `pub mod http;`)

**Interfaces:**
- Consumes: `Request`, `Response` (Task 4); `PullRequestSource`, models (Task 3); `RepoSlug` (Task 1).
- Produces:
  - `worker::spawn(source: Arc<dyn PullRequestSource>) -> (Sender<Request>, Receiver<Response>)`; the thread exits when the request sender is dropped; errors become `format!("{err:#}")`.
  - `HttpGitHub::new(repo: RepoSlug, token: String) -> anyhow::Result<HttpGitHub>` implementing `PullRequestSource`. Base URL `https://api.github.com`. Headers on every request: `Authorization: Bearer <token>`, `Accept: application/vnd.github+json`, `X-GitHub-Api-Version: 2022-11-28`, `User-Agent: reviewy`. Paths: `/repos/{owner}/{name}/pulls?state=open&sort=updated&direction=desc&per_page=100&page={n}` and `/repos/{owner}/{name}/pulls/{number}/files?per_page=100&page={n}`. A private `get_all_pages<T: DeserializeOwned>(&self, path_and_query: &str) -> anyhow::Result<Vec<T>>` fetches pages from 1 until a page returns fewer than 100 items or page 10 is reached.
  - Private wire types `WirePullRequest { number, title, draft, user: WireUser { login }, head: WireRef, base: WireRef }` where `WireRef` has `#[serde(rename = "ref")] name: String`, and `WireFile { filename, status, additions, deletions, patch: Option<String> }`, each with `impl From<Wire…> for …`. `draft` uses `#[serde(default)]`.

- [ ] **Step 1: Write the failing tests**

In `worker.rs`, with a `FakeSource { prs: Vec<PullRequest>, fail: bool }` implementing the trait:
```rust
#[test] fn serves_pull_request_requests()   // send LoadPullRequests → recv_timeout(2s) is PullRequests(Ok(v)) with v.len()==1
#[test] fn serves_file_requests_with_number() // send LoadFiles{number:7} → Files{number:7, result:Ok(_)}
#[test] fn reports_errors_as_strings()      // fail=true → PullRequests(Err(s)) with s containing "boom"
```
In `github/http.rs`:
```rust
#[test] fn converts_wire_pull_request()  // serde_json::from_str of a minimal PR JSON (number 5, title "T", user.login "ann", head.ref "feat", base.ref "main", no draft field) → PullRequest { number: 5, author: "ann", head_ref: "feat", base_ref: "main", draft: false, .. }
#[test] fn converts_wire_file_without_patch() // JSON with status "removed" and no patch → ChangedFile { status: Removed, patch: None, .. }
#[test] #[ignore] fn live_list_open_pull_requests() // HttpGitHub::new(RepoSlug::parse("ratatui/ratatui"), auth::resolve_token()) .list_open() is Ok and non-empty
```

- [ ] **Step 2: Run `cargo test worker:: http::`** — Expected: compile failure.

- [ ] **Step 3: Implement both files.** Build the reqwest client once in `new` with `reqwest::blocking::Client::builder().user_agent("reviewy")`. Wrap failures with `.with_context(|| format!("listing pull requests for {repo}"))` / `("loading files for #{number}")`. Use `error_for_status()` so 401/404 surface as errors.

- [ ] **Step 4: Run `cargo test`** — Expected: all pass. Then `cargo test -- --ignored live_list_open` — Expected: 1 passed (needs network and the gh login on this machine).

- [ ] **Step 5: Commit** — `git add -A && git commit -m "feat: add background worker and GitHub HTTP client"`

---

### Task 8: Wiring, event loop, and README

**Files:**
- Modify: `src/main.rs`
- Create: `README.md`

**Interfaces:**
- Consumes: everything above.
- Produces: `fn run(terminal: &mut ratatui::DefaultTerminal, app: &mut App, requests: &Sender<Request>, responses: &Receiver<Response>) -> anyhow::Result<()>`.

- [ ] **Step 1: Implement `main`**

Order: handle `-h`/`--help` by printing `usage: reviewy [owner/name]` plus a line on token sources and exiting 0; resolve `RepoSlug` from `args().nth(1)` or `detect_from_git`; resolve the token; build `HttpGitHub`; `worker::spawn`; `App::new`; send `Request::LoadPullRequests`; only then `ratatui::init()`. Any error before init prints `reviewy: {err:#}` to stderr and exits 1. Always call `ratatui::restore()` after `run`, then propagate `run`'s result.

`run` loop: `terminal.draw(|f| ui::render(f, app))`; `event::poll(Duration::from_millis(50))`; on `Event::Key` with `kind == KeyEventKind::Press` map through `keymap::action_for` then `app.dispatch`, sending any `Request`; drain `responses.try_recv()` into `app.apply`; break when `app.should_quit`.

- [ ] **Step 2: Verify the build and the error paths**

Run: `cargo build 2>&1 | tail -3` — Expected: `Finished`, no warnings.
Run: `cargo run -q -- not-a-slug; echo "exit=$?"` — Expected: `reviewy: ...` on stderr and `exit=1`, terminal untouched.
Run: `cd /tmp && /Users/albertoromero/development/reviewy/target/debug/reviewy; echo "exit=$?"` — Expected: message about no origin remote, `exit=1`.
Run: `GITHUB_TOKEN= GH_TOKEN= PATH=/usr/bin:/bin cargo run -q -- ratatui/ratatui; echo "exit=$?"` — Expected: the exact token error from Task 3, `exit=1`.

- [ ] **Step 3: Verify the TUI live** — use the `run` skill (or a pty via `script`) to start `cargo run -- ratatui/ratatui`, confirm PRs render, press `Enter`, confirm files and a coloured diff render, press `q`, confirm the shell prompt is intact.

- [ ] **Step 4: Write `README.md`** — what it is, install (`cargo install --path .`), usage line, token sources in order, the key table from the spec.

- [ ] **Step 5: Run `cargo clippy --all-targets -- -D warnings` and `cargo test`** — Expected: clean, all pass.

- [ ] **Step 6: Commit** — `git add -A && git commit -m "feat: wire event loop and add README"`
