# Reviewy: design

## Intent

A terminal app for reading GitHub pull request diffs for one repository
without leaving the terminal. The left pane lists open pull requests. The
right pane shows the changed files of the selected pull request and the
diff of the selected file. Written in Rust with Ratatui.

The user asked for exactly this and chose the defaults offered: PRs come
from one repo, the diff is read-only, auth borrows the token the GitHub CLI
already holds.

## Goals

- List open pull requests for one repository, newest activity first.
- Show the changed files of the selected PR with added/removed line counts.
- Show the unified diff of the selected file with added, removed and hunk
  header lines coloured.
- Keyboard driven, vim-style keys, never blocks the UI while fetching.
- Authenticate with `GITHUB_TOKEN`, `GH_TOKEN`, or `gh auth token`, in that
  order.
- Pick the repository from an `owner/name` argument or, failing that, the
  `origin` remote of the current directory.

## Non-goals (this version)

- Commenting, approving, merging.
- Closed or merged PRs, PRs across several repos.
- Language syntax highlighting, side-by-side diffs.
- Its own OAuth flow.

## Anti-goals

- Freezing the UI while a request is in flight.
- Printing the token anywhere.
- Crashing on a PR that has binary files, files without a patch, an empty
  file list, or no open PRs at all.

## Constraints

- Rust 2024 edition, stable toolchain (cargo 1.100 on this machine).
- Crates: ratatui 0.30, reqwest 0.13 (blocking + json + native-tls, because the rustls default needs cmake and this machine has none), serde 1 (derive),
  serde_json 1, anyhow 1. Crossterm is used through `ratatui::crossterm` so
  the two never drift apart.
- Networking runs on one background thread; the UI thread talks to it over
  `std::sync::mpsc` channels. No async runtime.
- The GitHub REST API is the data source: `GET /repos/{owner}/{name}/pulls`
  and `GET /repos/{owner}/{name}/pulls/{number}/files`. The `files`
  endpoint returns a per-file `patch`, so the app never parses a whole-PR
  diff.

## Approaches considered

1. **Shell out to `gh` for everything** (`gh pr list --json`, `gh pr diff`).
   Zero HTTP code, but the app then needs `gh` at runtime for every call,
   each call pays a process spawn, and the diff comes back as one blob that
   has to be split into files.
2. **Direct REST calls with the token `gh` already holds.** (Chosen.) One
   small HTTP client, typed JSON, per-file patches from the API, `gh` only
   consulted once at startup for the token and optional for users who set
   `GITHUB_TOKEN`.
3. **octocrab with tokio.** Typed and complete, but pulls an async runtime
   into a program whose UI loop is synchronous, for two endpoints.

## Decided design

### Layout

```
┌ Pull Requests · owner/name ─────┐┌ Files (3) ──────────────────────────┐
│ > #123 Fix login redirect  @ann ││ M src/auth.rs            +12  -3    │
│   #122 Add dark mode       @bob ││ A src/theme.rs           +80  -0    │
│   #120 [draft] Refactor db @ann ││ D src/old.rs              +0 -41    │
│                                 │├ #123 src/auth.rs ───────────────────┤
│                                 ││ @@ -10,7 +10,8 @@ fn login()        │
│                                 ││  fn login() {                       │
│                                 ││ -    let x = 1;                     │
│                                 ││ +    let x = 2;                     │
└─────────────────────────────────┘└─────────────────────────────────────┘
 j/k move  h/l/Tab pane  Enter open  r refresh  q quit        Loading #123…
```

Left pane is 35% of the width. The files list is as tall as it needs to be,
between 3 and 12 rows including its border. The focused pane has a
highlighted border. The footer shows key hints on the left and the current
status (loading, error, or empty message) on the right.

### Keys

| Key | In any pane | In PR list | In files | In diff |
|-----|-------------|------------|----------|---------|
| `q` | quit | | | |
| `Tab` / `Shift-Tab` | next / previous pane | | | |
| `h` / `l` | pane left / right | | | |
| `j` / `k`, arrows | | move selection | move selection, reload diff | scroll one line |
| `g` / `G` | | first / last | first / last | top / bottom |
| `Ctrl-d` / `Ctrl-u`, PageDown / PageUp | | | | scroll 20 lines |
| `Enter` | | load files for the PR, focus files | focus diff | |
| `r` | reload PR list | | | |

### Modules

- `repo.rs`: `RepoSlug` and how it is parsed from an argument or a remote
  URL.
- `auth.rs`: token resolution.
- `github/mod.rs`: `PullRequest`, `ChangedFile`, and the `PullRequestSource`
  trait the rest of the app depends on.
- `github/http.rs`: the reqwest implementation of that trait, with private
  wire structs so serde stays out of the public models.
- `diff.rs`: turns a patch string into typed lines.
- `app.rs`: all state and the rules for changing it. No I/O, no terminal
  types. Takes `Action`s and `Response`s, returns `Request`s.
- `keymap.rs`: key event to `Action`.
- `worker.rs`: the background thread that serves `Request`s.
- `ui.rs`: draws an `App` into a frame.
- `main.rs`: wiring and the event loop.

### Left to the builder

Exact colours, truncation of long titles, the wording of error messages
beyond what the plan pins.
