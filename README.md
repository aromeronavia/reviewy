# reviewy

A terminal UI for reading GitHub pull request diffs without leaving the shell.
The left pane lists a repository's open pull requests. The right pane shows
the selected pull request's changed files and the diff of the selected file.

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

Built with Rust and [Ratatui](https://ratatui.rs). Read-only: it does not
comment, approve, or merge.

## Install

```sh
cargo install --path .
```

Requires a Rust toolchain. On macOS nothing else is needed. On Linux the
`native-tls` backend needs the OpenSSL development headers.

## Usage

```sh
reviewy [owner/name]
```

Without an argument, reviewy reads the repository from the `origin` remote of
the current directory.

The GitHub token is the first of:

1. `GITHUB_TOKEN`
2. `GH_TOKEN`
3. `gh auth token`, the GitHub CLI's stored login

## Keys

| Key | Any pane | Pull requests | Files | Diff |
|-----|----------|---------------|-------|------|
| `q`, `Ctrl-c` | quit | | | |
| `Tab` / `Shift-Tab`, `l` / `h` | next / previous pane | | | |
| `j` / `k`, arrows | | move selection | move selection, show its diff | scroll one line |
| `g` / `G`, `Home` / `End` | | first / last | first / last | top / bottom |
| `Ctrl-d` / `Ctrl-u`, `PageDown` / `PageUp` | | | | scroll 20 lines |
| `Enter` | | load the PR's files | focus the diff | |
| `r` | reload the PR list | | | |

## Development

```sh
cargo test                                   # unit tests, no network
cargo test -- --ignored live_list_open_pull_requests   # one call to api.github.com
cargo clippy --all-targets -- -D warnings
```

The design spec and implementation plan live under `docs/superpowers/`.
