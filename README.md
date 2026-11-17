# reviewy

Review GitHub pull requests without leaving the terminal. The sidebar lists a
repository's open pull requests as cards with their CI state, review
decision, and comment count. The right side shows either the changes, with
review threads inline under the lines they belong to, or the conversation:
the description, every comment and review, and threads that no longer map
onto the diff. You can leave line comments, post PR comments, and submit a
review with a verdict.

```
┌ Pull Requests · owner/name ─┐ Changes │ Conversation
│ #123 Fix login redirect     │┌ Files (3) ────────────────────────────────┐
│  ann · feat/login → main    ││ M src/auth.rs                 +12  -3     │
│  ✓ ci  ○ review  2 💬       ││ A src/theme.rs                +80  -0     │
│                             │├ #123 src/auth.rs ─────────────────────────┤
│ #122 Add dark mode          ││ @@ -10,7 +10,8 @@                         │
│  bob · dark-mode → main     ││  fn login() {                             │
│  ✗ ci  ✓ approved           ││-    let x = 1;                            │
│                             ││+    let x = 2;                            │
│                             ││    ann · 2026-11-10: why 2?               │
│                             ││    pending: because of #99                │
└─────────────────────────────┘└───────────────────────────────────────────┘
 #123 · in src/auth.rs:12 · 1 pending              Review submitted for #123
 j/k move  c comment line  R review  v conversation  Esc back  q quit
```

Built with Rust and [Ratatui](https://ratatui.rs).

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

Set `GITHUB_API_URL` to use a GitHub Enterprise server.

## Reviewing

- Press `Enter` on a pull request to load its files and conversation. `v`
  switches the right side between Changes and Conversation.
- In the diff, `j`/`k` move a cursor. `c` on a line opens a comment box;
  sending it with `Ctrl-s` adds the comment to a pending review, shown
  inline in yellow. Pending comments stay local until you submit a review.
- `c` anywhere else opens a comment on the pull request itself, which is
  posted as soon as you press `Ctrl-s`.
- `R` submits a review: pick approve, request changes, or comment, write a
  body, press `Ctrl-s`. Pending line comments go with it. Request changes
  needs a body; a comment review needs a body or pending comments; approve
  needs neither.
- Nothing is sent to GitHub without `Ctrl-s` in a box whose title says what
  will be sent and where. `Esc` cancels.
- The status bar's first row says where you are and how many comments are
  pending; the second row lists the keys that work right now.

## Keys

| Key | Browse | Pick a verdict | Compose |
|-----|--------|----------------|---------|
| `q`, `Ctrl-c` | quit | quit (`Ctrl-c`) | quit (`Ctrl-c`) |
| `Esc` | back: diff → files → pull requests | cancel | cancel |
| `Tab` / `l`, `Shift-Tab` / `h` | next / previous pane | | |
| `v` | switch Changes / Conversation | | |
| `j`/`k`, arrows | move or scroll in the focused pane | | |
| `g`/`G`, `Home`/`End` | first / last | | |
| `Ctrl-d`/`Ctrl-u`, `PgDn`/`PgUp` | 20 rows in the diff or conversation | | |
| `Enter` | open the pull request, or enter the file | | newline |
| `c` | diff: line comment; elsewhere: PR comment | comment review | |
| `R` | pick a review verdict | | |
| `a` / `r` | | approve / request changes | |
| `r` | reload the pull request list | | |
| `Ctrl-s` | | | send |
| printable, `Backspace` | | | edit |

## How it talks to GitHub

The pull request list and each conversation come from one GraphQL request
each, which is how review decisions and CI state arrive without a call per
pull request. Diffs come from the REST `pulls/{n}/files` endpoint, the only
one that returns patches. Comments and reviews are posted with the REST
`issues/{n}/comments` and `pulls/{n}/reviews` endpoints.

## Development

```sh
cargo test                                   # unit tests, no network
cargo test -- --ignored live_                # two read-only calls to api.github.com
cargo clippy --all-targets -- -D warnings
```

The design specs and implementation plans live under `docs/superpowers/`.
