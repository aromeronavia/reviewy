# Reviewy v2: reviewing from the terminal

## Intent

Turn reviewy from a diff viewer into a place to review: read the PR
description and conversation, see review comments where they belong in the
diff, leave comments, and submit a review. The sidebar becomes cards that
show each PR's state at a glance, and the footer becomes a status bar that
always says where you are.

The user asked for, in their words: add comments, add review, a status bar
that shows whether a file is entered, Escape to exit the file, PR
description, comments, status in the sidebar, and cards instead of a text
list. They chose the proposed reading: comments are both read and written,
line comments on the diff plus PR-level comments, and reviews are approve,
request changes, or comment.

## Goals

- Escape backs out one level: diff to files, files or conversation to the
  PR list.
- A two-row status bar: row one says the context (PR list, files of #N,
  inside a file at a line, conversation of #N, composing), the number of
  pending line comments, and the latest outcome or error; row two lists the
  keys that work right now.
- The right side has two tabs, Changes and Conversation. Conversation shows
  the PR description, then every PR comment and review in time order, then
  review threads that no longer map onto the diff.
- Review threads that do map onto the current file appear inline under
  their diff line, in a distinct colour, with author and date.
- The diff pane has a cursor line. `c` on a diff line composes a line
  comment, which joins a local pending review. Pending comments show inline
  too, marked as pending.
- `c` anywhere else composes a PR-level comment that posts immediately.
- `R` submits a review: pick approve, request changes, or comment, write a
  body, send. Pending line comments go with it. Request changes requires a
  body; comment requires a body or pending comments; approve needs neither.
- Sidebar cards: number and title; author, head and base branch; checks
  state, review decision, draft marker, comment count.
- Composing happens in a small overlay text box: typed characters, Enter
  for a newline, Backspace, Ctrl-s to send, Escape to cancel.

## Non-goals

- Editing or deleting comments, replying inside a thread, resolving threads.
- Markdown rendering; bodies are shown as plain text, wrapped.
- Multi-line (range) comments, suggestions, emoji reactions.
- Merging, closing, re-requesting review.
- A cursor inside the text box; typing appends at the end.

## Anti-goals

- Posting anything without the user pressing Ctrl-s in a box whose title
  says what will be sent and to which PR.
- Losing pending line comments on a failed submit; they stay until a
  submit succeeds.
- A stale response from a PR the user left replacing what they now see.
- Any wrapped or truncated line panicking at narrow widths.

## Constraints

- Same stack as v1: Rust 2024, ratatui 0.30 with crossterm via
  `ratatui::crossterm`, reqwest 0.13 (blocking, json, native-tls), serde,
  serde_json, anyhow. No new crates; the text box is hand-rolled because
  the published text-area crate targets the previous ratatui.
- Reads of the PR list and the conversation use the GitHub GraphQL API
  (`POST /graphql`), validated on 2026-11-17 against ratatui/ratatui:
  - list: `pullRequests(states: OPEN, orderBy: UPDATED_AT desc)` with
    `number title isDraft reviewDecision author{login} headRefName
    baseRefName headRefOid comments{totalCount}
    commits(last:1){nodes{commit{statusCheckRollup{state}}}}` and cursor
    pagination through `pageInfo`.
  - conversation: `pullRequest(number){ body comments{nodes{author{login}
    body createdAt}} reviews{nodes{author{login} state body createdAt}}
    reviewThreads{nodes{path line diffSide isOutdated isResolved
    comments{nodes{author{login} body createdAt}}}} }`.
- Diffs stay on REST (`pulls/{n}/files`), the only API that returns
  patches. Writes use REST: `POST issues/{n}/comments` with `{body}` and
  `POST pulls/{n}/reviews` with `{event, body, commit_id, comments:[{path,
  line, side, body}]}`.
- The side of a line comment comes from the diff line: added and context
  lines are RIGHT with the new line number, removed lines are LEFT with the
  old line number, hunk headers cannot be commented.
- `app.rs` stays free of ratatui and reqwest. The UI reports the sizes it
  drew with (conversation width and height, diff height) back to the app
  after each frame so scrolling and cursor-following are exact and
  testable; before the first frame the app assumes 80 by 20.

## Approaches considered

1. **REST for everything.** Review decision and checks would need two extra
   calls per PR for the list, and comments need three endpoints per PR.
   Rejected: a 30-PR list would cost 60 requests on every refresh.
2. **GraphQL for reads, REST for diffs and writes.** (Chosen.) One request
   per page of PRs with status included, one request per PR for the whole
   conversation, and the two write endpoints are plain JSON posts.
3. **Everything through the `gh` CLI.** Keeps auth trivial but each action
   spawns a process, output has to be re-parsed, and line comments need
   the review API anyway.

## Decided design

### Layout

```
┌ Pull Requests · owner/name ─┐ Changes │ Conversation
│ #123 Fix login redirect     │┌ Files (3) ────────────────────────────────┐
│   ann · feat/login → main   ││ M src/auth.rs                 +12  -3     │
│   ✓ checks  ○ review  2 💬  ││ A src/theme.rs                +80  -0     │
│                             │├ #123 src/auth.rs ─────────────────────────┤
│ #122 Add dark mode          ││ @@ -10,7 +10,8 @@                         │
│   bob · dark-mode → main    ││  fn login() {                             │
│   ✗ checks  ✓ approved      ││-    let x = 1;                            │
│                             ││+    let x = 2;                            │
│                             ││    ann · 2026-11-10: why 2?               │
│                             ││    (pending) me: because of #99           │
└─────────────────────────────┘└───────────────────────────────────────────┘
 #123 · in src/auth.rs:12 · 1 pending comment              Comment posted
 j/k move  c comment  R review  v conversation  Esc back  q quit
```

The sidebar is at least 32 columns. Each card is three lines plus a blank
separator; the selected card has a highlighted background. Markers: checks
`✓` green, `✗` red, `●` yellow pending, nothing when there are no checks;
review `✓ approved` green, `✗ changes` red, `○ review` yellow for review
required, nothing otherwise; `draft` dim; `N 💬` dim when N > 0.

### Keys

| Key | Browse | Pick review | Compose |
|-----|--------|-------------|---------|
| `q`, `Ctrl-c` | quit | | |
| `Esc` | back one level | cancel | cancel |
| `Tab` / `l`, `Shift-Tab` / `h` | next / previous pane | | |
| `v` | switch Changes / Conversation | | |
| `j`/`k`, arrows, `g`/`G`, `Ctrl-d`/`Ctrl-u`, PgDn/PgUp | move cursor or scroll in the focused pane | | |
| `Enter` | PR list: open; files: enter the file | | newline |
| `c` | diff: line comment; elsewhere with a PR open: PR comment | comment review | |
| `R` | pick a review verdict | | |
| `a` / `r` | | approve / request changes | |
| `r` | reload the PR list | | |
| `Ctrl-s` | | | send |
| printable, Backspace | | | edit |

Pane order for Tab: PR list, then the visible tab's panes (Files then
Diff, or Conversation). Escape from Diff goes to Files; from Files or
Conversation to the PR list.

### Modules

- `diff.rs`: lines now carry `old_line` and `new_line`.
- `github/mod.rs`: `PullRequest` gains `head_sha`, `review_decision`,
  `checks`, `comment_count`; new `Conversation`, `Comment`, `Thread`,
  `ReviewSubmission`, `NewLineComment`, `DiffSide`, `ReviewEvent`; the
  trait gains `conversation`, `post_comment`, `submit_review`.
- `github/http.rs`: a private `graphql` helper, GraphQL wire structs for
  the two queries, REST posts for the two writes, status-string mapping
  kept private here.
- `app.rs`: `Focus` gains `Conversation`; `Tab`, `Mode`, `Compose`,
  `ComposeTarget`, `InFlight`, `Notice`, `Viewport`, `DiffRow` are new;
  `Status` is replaced by `in_flight` plus `notice`; `apply` returns
  follow-up requests; a small word-wrap helper feeds the conversation
  pane.
- `keymap.rs`: `action_for(key, &Mode)`.
- `ui.rs`: cards, tab bar, conversation pane, diff rows with cursor and
  comment rows, two-row status bar, compose and pick overlays; `render`
  returns the `Viewport` it drew with.
- `worker.rs`: serves the three new requests.
- `main.rs`: feeds the viewport back after each frame.

### Left to the builder

Exact colours and marker glyphs, the wording of notices, the overlay
size, how a comment body longer than the box is shown while typing.
