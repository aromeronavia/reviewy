# Reviewy v2 (review features) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add PR description and conversation, inline and pending line comments, PR comments, review submission, Escape navigation, a two-row status bar, and status cards in the sidebar.

**Architecture:** Same shape as v1. `App` stays a pure state machine but gains a `Mode` (browse, pick verdict, compose), a `Tab` for the right side, a cursor-based diff built from rows, in-flight tracking instead of a status string, and `dispatch`/`apply` both return the requests to send. Reads of PRs and conversations move to GraphQL; diffs and writes stay on REST. The UI returns the sizes it drew with so the app scrolls exactly.

**Tech Stack:** unchanged (Rust 2024, ratatui 0.30, reqwest 0.13 blocking+json+native-tls, serde, serde_json, anyhow). No new crates.

**Spec:** `docs/superpowers/specs/2026-11-17-reviewy-review-design.md`

## Global Constraints

- No new dependencies. Crossterm only via `ratatui::crossterm`.
- `app.rs` imports nothing from `ratatui` or `reqwest`.
- Nothing is posted to GitHub without `Action::Submit` from a compose box; the live verification in Task 10 exercises only reads.
- The token is never printed, logged, or put in an error message.
- GraphQL field names exactly as in the spec's Constraints section (validated live on 2026-11-17).
- Every test from v1 that still describes wanted behaviour keeps passing; tests that asserted the removed `Status` type are rewritten in Task 4, not deleted.
- Commit after every task with a conventional-commit message.

## Review Focus

1. A review submit that fails (422, network) keeps the pending line comments and shows the error; nothing is lost. Test in Task 6 (`failed_review_keeps_pending_comments`).
2. `c` on a hunk header, a comment row, or with no PR open is refused with a notice and never panics. Test in Task 6 (`comment_on_hunk_header_is_refused`, `comment_without_open_pr_is_refused`).
3. A PR body that is empty, or has a 500-character line, renders at 20 columns without panic. Test in Task 4 (`wrap_splits_words_longer_than_width`, `rows_for_empty_body`) and Task 8 (`renders_conversation_in_tiny_terminal`).
4. Files or conversation for PR A arriving after the user opened PR B are dropped. Test in Task 4 (`stale_conversation_is_dropped`, `stale_files_response_is_ignored`).
5. GraphQL nulls for review decision, checks, author, and body map to `None`/empty and cards render without markers. Test in Task 2 (`maps_null_decision_checks_and_author`) and Task 8 (`card_without_checks_or_reviews_has_no_markers`).

---

### Task 1: Diff line numbers

**Files:**
- Modify: `src/diff.rs`

**Interfaces:**
- Produces: `DiffLine { pub kind, pub text, pub old_line: Option<u64>, pub new_line: Option<u64> }`. `parse_patch` fills the numbers from `@@ -a[,b] +c[,d] @@` headers: context lines get both, added only `new_line`, removed only `old_line`, hunk headers and `\` marker lines neither and no advance.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn numbers_context_added_and_removed_lines()  // "@@ -10,3 +20,3 @@\n a\n-b\n+c\n d" → (Some(10),Some(20)), (Some(11),None), (None,Some(21)), (Some(12),Some(22))
#[test] fn second_hunk_restarts_numbering()            // two hunks; first line after "@@ -50 +60 @@" is (Some(50),Some(60))
#[test] fn hunk_and_no_newline_marker_have_no_numbers() // both (None,None); the marker does not advance the next line
#[test] fn hunk_without_counts_parses()                // "@@ -1 +1 @@\n-x\n+y" → x old 1, y new 1
```

- [ ] **Step 2: Run `cargo test diff::`** — Expected: compile failure on the new fields.
- [ ] **Step 3: Implement** with a private `fn hunk_start(header: &str) -> Option<(u64, u64)>` and a running `(old, new)` pair in `parse_patch`. Existing constructors in other files' tests must add the two fields (`old_line: None, new_line: None`).
- [ ] **Step 4: Run `cargo test`** — Expected: all pass (existing 53 plus 4).
- [ ] **Step 5: Commit** — `git add -A && git commit -m "feat: number diff lines from hunk headers"`

---

### Task 2: Models and GraphQL reads

**Files:**
- Modify: `src/github/mod.rs`, `src/github/http.rs`

**Interfaces:**
- Produces in `github/mod.rs` (all `Debug, Clone, PartialEq, Eq`; enums also `Copy`):
  - `PullRequest { number: u64, title: String, author: String, head_ref: String, base_ref: String, head_sha: String, draft: bool, review_decision: ReviewDecision, checks: ChecksState, comment_count: u64 }`
  - `enum ReviewDecision { Approved, ChangesRequested, ReviewRequired, None }`, `enum ChecksState { Success, Failure, Pending, None }`, `enum ReviewState { Approved, ChangesRequested, Commented, Dismissed, Pending }`, `enum DiffSide { Left, Right }`, `enum ReviewEvent { Approve, RequestChanges, Comment }`
  - `Comment { author: String, created_at: String, body: String, review_state: Option<ReviewState> }`
  - `Thread { path: String, line: Option<u64>, side: DiffSide, outdated: bool, resolved: bool, comments: Vec<Comment> }`
  - `Conversation { body: String, comments: Vec<Comment>, threads: Vec<Thread> }` (also `Default`)
  - `NewLineComment { path: String, line: u64, side: DiffSide, body: String }`, `ReviewSubmission { event: ReviewEvent, body: String, commit_sha: String, comments: Vec<NewLineComment> }`
  - trait `PullRequestSource` gains `fn conversation(&self, number: u64) -> Result<Conversation>; fn post_comment(&self, number: u64, body: &str) -> Result<()>; fn submit_review(&self, number: u64, review: &ReviewSubmission) -> Result<()>;` (the two writes are implemented in Task 3; add them to the trait here with `todo!()` bodies in `HttpGitHub` only until Task 3).
- Produces in `http.rs`:
  - `HttpGitHub::with_base_url(self, base_url: String) -> HttpGitHub` (replaces the `BASE_URL` const with a field; default `https://api.github.com`).
  - private `fn graphql<T: DeserializeOwned>(&self, query: &str, variables: serde_json::Value) -> Result<T>` posting `{query, variables}` to `{base}/graphql`; a GraphQL `errors` array becomes `bail!("GitHub: {first message}")`.
  - `const LIST_QUERY: &str` and `const CONVERSATION_QUERY: &str` exactly per the spec fields; list pages with `first: 50` and `after`, stopping on `hasNextPage == false` or after 20 pages.
  - private wire structs with `#[serde(rename_all = "camelCase")]`, and private mappers `fn review_decision(s: Option<&str>) -> ReviewDecision`, `fn checks_state(s: Option<&str>) -> ChecksState`, `fn review_state(s: &str) -> ReviewState`, `fn diff_side(s: &str) -> DiffSide`, `fn conversation_from_wire(w: WireConversation) -> Conversation` (comments and reviews merged and sorted by `created_at`; a review with empty body and state `COMMENTED` or `PENDING` is dropped).

- [ ] **Step 1: Write the failing tests** (fixture JSON strings inline, minimal but with every field the wire structs read)

```rust
#[test] fn maps_list_node_to_pull_request()        // SUCCESS + APPROVED + isDraft true + comments.totalCount 2 + headRefOid
#[test] fn maps_null_decision_checks_and_author()  // reviewDecision null, statusCheckRollup null, author null → None, None, "ghost"
#[test] fn maps_conversation_with_threads()        // body, one comment, one APPROVED review with body, one thread LEFT line 7 outdated=false → comments sorted, thread side Left
#[test] fn drops_empty_commented_reviews()         // COMMENTED review with "" body is not in comments
#[test] fn graphql_errors_become_errors()          // parse `{"errors":[{"message":"boom"}]}` through the response type → Err containing "boom" (test the parse+check helper, not the network)
#[test] #[ignore] fn live_list_open_pull_requests()  // (update) first PR has non-empty head_sha
#[test] #[ignore] fn live_conversation_has_threads() // ratatui/ratatui #2266 → threads non-empty
```

- [ ] **Step 2: Run `cargo test http::`** — Expected: compile failure.
- [ ] **Step 3: Implement.** Update the v1 `WirePullRequest` tests to the GraphQL shape (REST PR wire structs are deleted; the REST files endpoint stays). `worker.rs`'s `FakeSource` and `app.rs`/`ui.rs` test helpers gain the new `PullRequest` fields and the three new trait methods (`unimplemented!()` is fine in the fake for now).
- [ ] **Step 4: Run `cargo test`** then `cargo test -- --ignored live_` — Expected: all pass; both live tests pass.
- [ ] **Step 5: Commit** — `git commit -am "feat: fetch pull requests and conversations over GraphQL"` (add first).

---

### Task 3: Writes and worker

**Files:**
- Modify: `src/github/http.rs`, `src/worker.rs`, `src/app.rs` (only the `Request`/`Response` enums, see Interfaces)

**Interfaces:**
- Produces in `app.rs`: `Request::{LoadConversation { number: u64 }, PostComment { number: u64, body: String }, SubmitReview { number: u64, review: ReviewSubmission }}` and `Response::{Conversation { number: u64, result: Result<Conversation, String> }, CommentPosted { number: u64, result: Result<(), String> }, ReviewSubmitted { number: u64, result: Result<(), String> }}`. No other `app.rs` change in this task.
- Produces in `http.rs`: private `fn comment_payload(body: &str) -> serde_json::Value` (`{"body": …}`), `fn review_payload(review: &ReviewSubmission) -> serde_json::Value` (`event` is `APPROVE`/`REQUEST_CHANGES`/`COMMENT`; `commit_id`; `body` only when non-empty; `comments` only when non-empty, each `{path, line, side: "LEFT"|"RIGHT", body}`), private `fn post(&self, path: &str, payload: &serde_json::Value) -> Result<()>` using `describe_status` on failure; `post_comment` → `/repos/{o}/{n}/issues/{number}/comments`; `submit_review` → `/repos/{o}/{n}/pulls/{number}/reviews`.
- Produces in `worker.rs`: `serve` handles the three new requests.

- [ ] **Step 1: Write the failing tests**

```rust
// http.rs
#[test] fn review_payload_omits_empty_body_and_comments()  // Approve, "" → {"event":"APPROVE","commit_id":"abc"} exactly
#[test] fn review_payload_includes_line_comments()         // RequestChanges with one Left comment → "side":"LEFT", "line":7
#[test] fn posts_comment_to_issues_endpoint()  // stub server on 127.0.0.1:0 records one request; assert it starts with "POST /repos/o/n/issues/5/comments" and body contains "\"body\":\"hi\""
#[test] fn posts_review_to_reviews_endpoint()  // same stub; path ".../pulls/5/reviews", body contains "REQUEST_CHANGES"
// worker.rs (FakeSource records calls in a Mutex<Vec<String>>)
#[test] fn serves_conversation_requests()      // LoadConversation{7} → Response::Conversation{number:7, result:Ok(_)}
#[test] fn serves_comment_and_review_requests() // PostComment → CommentPosted{Ok}; SubmitReview → ReviewSubmitted{Ok}; fake recorded both
```

The stub: `std::net::TcpListener::bind("127.0.0.1:0")`, a thread that accepts one connection, reads until `\r\n\r\n` then `Content-Length` bytes, replies `HTTP/1.1 201 Created\r\nContent-Length: 2\r\n\r\n{}`, and sends the raw request text over an `mpsc` channel. Client is `HttpGitHub::new(RepoSlug::parse("o/n"), "t".into()).with_base_url(format!("http://{addr}"))`.

- [ ] **Step 2: Run `cargo test`** — Expected: compile failure.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run `cargo test`** — Expected: all pass.
- [ ] **Step 5: Commit** — `git add -A && git commit -m "feat: post comments and reviews; serve them from the worker"`

---

### Task 4: App core: tabs, escape, conversation, in-flight tracking

**Files:**
- Modify: `src/app.rs`

**Interfaces:**
- Consumes: Task 2 models, Task 3 request/response variants.
- Produces (replacing `Status`, `pending_pr`, `diff`, `diff_scroll`, `PAGE`):
  - `enum Focus { PullRequests, Files, Diff, Conversation }`, `enum Tab { Changes, Conversation }`
  - `enum Mode { Browse, PickReviewEvent, Compose(Compose) }`, `struct Compose { pub target: ComposeTarget, pub text: String }`, `enum ComposeTarget { PullRequestComment, LineComment { path: String, line: u64, side: DiffSide, excerpt: String }, Review { event: ReviewEvent } }` (defined now, driven in Task 6)
  - `enum InFlight { PullRequests, Files(u64), Conversation(u64), Comment(u64), Review(u64) }` (Copy, Eq)
  - `struct Notice { pub text: String, pub error: bool }`
  - `struct Viewport { pub conversation_width: usize, pub conversation_height: usize, pub diff_width: usize, pub diff_height: usize }` with `Default` = 80, 20, 80, 20
  - `enum Action { Quit, NextPane, PrevPane, Escape, Up, Down, Top, Bottom, PageUp, PageDown, Open, Refresh, ToggleTab, Comment, Review, Pick(ReviewEvent), Type(char), Newline, Backspace, Submit, Cancel }`
  - `const PAGE_LINES: usize = 20;` (private)
  - `App` fields: `repo, focus, tab, mode, prs, pr_index, opened: Option<u64>, files, file_index, loaded_pr: Option<u64>, conversation: Option<Conversation>, conversation_for: Option<u64>, conversation_scroll: usize, diff_rows: Vec<DiffRow>, diff_cursor: usize, diff_offset: usize, pending_comments: Vec<NewLineComment>, in_flight: Vec<InFlight>, notice: Option<Notice>, viewport: Viewport, should_quit` (all `pub`). `DiffRow` is defined in Task 5; in this task it is `pub enum DiffRow { Line(DiffLine) }` only.
  - `App::new(repo) -> App` starts `Browse`, `Changes`, no in-flight work.
  - `App::dispatch(&mut self, action: Action) -> Vec<Request>`, `App::apply(&mut self, response: Response) -> Vec<Request>`, `App::set_viewport(&mut self, v: Viewport)`, `App::selected_pr`, `App::selected_file`, `App::opened_pr(&self) -> Option<&PullRequest>`, `App::is_loading(&self, what: InFlight) -> bool`, `App::busy_message(&self) -> Option<String>` (first in-flight item: `Loading pull requests…`, `Loading #N…`, `Posting comment…`, `Submitting review…`).
  - `pub fn wrap(text: &str, width: usize) -> Vec<String>` (greedy by words per logical line; words longer than `width` are split; `width` 0 behaves as 1; an empty line stays one empty row)
  - `pub struct ConvRow { pub text: String, pub kind: ConvRowKind }`, `pub enum ConvRowKind { Title, Meta, Body, Heading, Author, Thread }`, `pub fn conversation_rows(number: u64, pr: Option<&PullRequest>, conversation: &Conversation, width: usize) -> Vec<ConvRow>`: title `#N title`, meta `author wants to merge head into base`, blank, body rows (or `No description.`), blank, heading `Comments (N)`, per comment: author row `author · YYYY-MM-DD` plus ` · approved`/` · changes requested`/` · commented` when it is a review, body rows, blank; then heading `Threads not on the current diff (N)` listing threads whose `line` is `None` or `outdated` as `path (outdated)` plus their comments.
  - `App::conversation_row_count(&self) -> usize` using the viewport width.

**Rules:**
- Pane cycle for `NextPane`/`PrevPane`: `[PullRequests, Files, Diff]` when `tab == Changes`, `[PullRequests, Conversation]` when `Conversation`; wraps.
- `Escape`: `Diff → Files`, `Files → PullRequests`, `Conversation → PullRequests`, else nothing. Any `dispatch` first clears `notice`.
- `ToggleTab` flips `tab`; focus `Files`/`Diff` becomes `Conversation` and vice versa; `PullRequests` stays.
- `Open` on `PullRequests`: `opened = Some(n)`; focus `Files` or `Conversation` by tab; returns `LoadFiles` unless `loaded_pr == Some(n)` or already in flight, and `LoadConversation` unless `conversation_for == Some(n)` or in flight; every returned request is recorded in `in_flight`. `Open` on `Files` → focus `Diff`. Otherwise nothing.
- `Refresh` → `[LoadPullRequests]` unless already in flight.
- Movement: `PullRequests` and `Files` as v1 (`Files` rebuilds rows via Task 5's hook; for now re-parses into `DiffRow::Line`); `Diff` moves `diff_cursor` over `diff_rows` then calls `follow_cursor` (keep the cursor at least 3 rows from the top/bottom edge of a `viewport.diff_height` window, offset clamped to `rows.len().saturating_sub(height)`); `Conversation` moves `conversation_scroll` clamped to `conversation_row_count().saturating_sub(viewport.conversation_height)`.
- `apply`: every response removes its `InFlight` entry. `PullRequests(Ok)` keeps selection by number as v1; `Err` → `notice` error. `Files`/`Conversation` for `number != opened` are dropped. `Files(Ok)` sets `files`, `loaded_pr`, `file_index = 0`, rebuilds rows, `diff_cursor = diff_offset = 0`. `Conversation(Ok)` sets `conversation`, `conversation_for`, `conversation_scroll = 0`, rebuilds rows. `CommentPosted`/`ReviewSubmitted` are handled in Task 6 (no-op here beyond in-flight removal).

- [ ] **Step 1: Rewrite the v1 app tests that referenced `Status`, `pending_pr`, `diff`, `diff_scroll`** to the new fields (same behaviours: `is_loading(InFlight::PullRequests)`, `notice.error`, `diff_rows.len()`, `diff_cursor`), and add:

```rust
#[test] fn starts_idle_and_refresh_starts_loading()        // new(): in_flight empty; Refresh → [LoadPullRequests], is_loading(PullRequests)
#[test] fn open_requests_files_and_conversation()           // → [LoadFiles{2}, LoadConversation{2}], both in flight, opened Some(2), focus Files
#[test] fn open_with_conversation_tab_focuses_conversation()
#[test] fn open_skips_already_loaded_parts()                // after files+conversation loaded for 2, reopen → []
#[test] fn stale_conversation_is_dropped()                  // opened 2, Conversation{1,Ok} → conversation None
#[test] fn stale_files_response_is_ignored()                // kept from v1, now via `opened`
#[test] fn escape_walks_back_to_the_pr_list()               // Diff→Files→PullRequests→PullRequests
#[test] fn escape_from_conversation_returns_to_pr_list()
#[test] fn toggle_tab_moves_focus_between_sides()           // Files→Conversation→Files; PullRequests stays
#[test] fn panes_cycle_per_tab()                            // Changes: 3 panes; Conversation: 2 panes
#[test] fn dispatch_clears_notice()
#[test] fn busy_message_names_first_in_flight_item()        // files + conversation in flight → "Loading #2…"
#[test] fn diff_cursor_follows_with_margin()                // 100 rows, height 10: cursor 0..=6 → offset 0; cursor 7 → offset 1; Bottom → offset 90; Top → 0
#[test] fn conversation_scroll_clamps_to_rows_minus_height()
#[test] fn wrap_breaks_on_words()                           // wrap("aa bb cc", 5) == ["aa bb", "cc"]
#[test] fn wrap_splits_words_longer_than_width()            // wrap("abcdefgh", 3) == ["abc","def","gh"]; wrap("", 10) == [""]
#[test] fn rows_for_empty_body()                            // body "" → contains "No description."
#[test] fn rows_order_comments_and_mark_reviews()           // approved review row contains "· approved"; comments sorted by created_at
#[test] fn rows_list_outdated_threads()                     // thread outdated → "src/a.rs (outdated)"
```

- [ ] **Step 2: Run `cargo test app::`** — Expected: compile failure.
- [ ] **Step 3: Implement.** `ui.rs` and `main.rs` will not compile until Tasks 8 and 10; to keep the suite runnable, make the minimal edits there in this task: `ui.rs` reads `app.diff_rows`/`diff_cursor`/`notice`/`busy_message()` in place of the removed fields, `main.rs` iterates the `Vec<Request>` from `dispatch`/`apply`. Keep those edits minimal; Tasks 8 to 10 replace them.
- [ ] **Step 4: Run `cargo test`** — Expected: all pass.
- [ ] **Step 5: Commit** — `git add -A && git commit -m "feat: tabs, escape, conversation view, and in-flight tracking in the app state"`

---

### Task 5: Diff rows with inline threads and pending comments

**Files:**
- Modify: `src/app.rs`

**Interfaces:**
- Produces: `pub enum DiffRow { Line(DiffLine), Comment { header: Option<String>, text: String, pending: bool } }`; `App::rebuild_diff_rows(&mut self)` (private, called whenever `files`, `file_index`, `conversation`, `pending_comments`, or `viewport` change); `App::commentable_at_cursor(&self) -> Option<(u64, DiffSide, String)>` (line number, side, excerpt for the `Line` row under the cursor: `Added`/`Context` → `Right` + `new_line`, `Removed` → `Left` + `old_line`; `None` for hunk headers, marker lines, comment rows, or no file).

**Rules:** after each diff line, append rows for every non-outdated thread whose `path == file.filename` and whose `(side, line)` matches that line, then rows for every `pending_comments` entry matching `(path, line, side)`. A comment's body is wrapped at `viewport.diff_width.saturating_sub(6).max(10)`; the first row carries `header: Some("author · YYYY-MM-DD")` (pending: `Some("pending")`), later rows `None`. `diff_cursor` is clamped after a rebuild.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn threads_appear_under_their_line()            // thread Right line 21 on "+c" (new_line 21) → next row is Comment{header Some("ann · 2026-01-02"), pending false}
#[test] fn left_side_threads_match_old_line_numbers()   // thread Left line 11 on "-b"
#[test] fn outdated_and_other_file_threads_are_skipped()
#[test] fn pending_comments_follow_existing_threads()   // pending row after thread rows, header Some("pending")
#[test] fn long_comment_bodies_wrap_into_rows()         // diff_width 30 → body of 60 chars gives 3+ rows, only first has header
#[test] fn commentable_at_cursor_reports_side_and_line() // on "+c" → (21, Right, "+c"); on "-b" → (11, Left, "-b"); on hunk → None; on a comment row → None
```

- [ ] **Step 2: Run `cargo test app::`** — Expected: compile failure.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run `cargo test`** — Expected: all pass.
- [ ] **Step 5: Commit** — `git add -A && git commit -m "feat: show review threads and pending comments inline in the diff"`

---

### Task 6: Compose flows: comments and reviews

**Files:**
- Modify: `src/app.rs`

**Interfaces:**
- Consumes: `Mode`, `Compose`, `ComposeTarget`, `commentable_at_cursor`, `Request::PostComment`/`SubmitReview`, `Response::CommentPosted`/`ReviewSubmitted`.
- Produces: the `Comment`, `Review`, `Pick`, `Type`, `Newline`, `Backspace`, `Submit`, `Cancel` arms of `dispatch`; `fn event_label(event: ReviewEvent) -> &'static str` (`Approve`, `Request changes`, `Comment`).

**Rules:**
- In `Browse`: `Comment` with focus `Diff` and `commentable_at_cursor() == Some(..)` → `Mode::Compose(LineComment{..})`; focus `Diff` otherwise → notice `This line cannot be commented` (not an error); any other focus with `opened == Some(_)` → `Compose(PullRequestComment)`; no PR open → notice `Open a pull request first`. `Review` with a PR open → `Mode::PickReviewEvent`, else that same notice.
- In `PickReviewEvent`: `Pick(e)` → `Compose(Review{event: e})`; `Cancel`/`Escape` → `Browse`.
- In `Compose`: `Type(c)` appends, `Newline` appends `\n`, `Backspace` pops one char, `Cancel`/`Escape` → `Browse` discarding text. `Submit` with trimmed text:
  - `PullRequestComment`: empty → notice error `Comment is empty`, stay; else `[PostComment{number: opened, body}]`, in flight `Comment(n)`, `Browse`.
  - `LineComment`: empty → same notice; else push `NewLineComment`, rebuild rows, notice `Pending comments: N`, `Browse`.
  - `Review{event}`: `RequestChanges` with empty body → notice error `Request changes needs a body`; `Comment` with empty body and no pending comments → `A comment review needs a body or line comments`; else `[SubmitReview{number, ReviewSubmission{event, body, commit_sha: opened_pr().head_sha, comments: pending.clone()}}]`, in flight `Review(n)`, `Browse`.
  - Movement and other browse actions are ignored while in `PickReviewEvent`/`Compose` (return `[]`).
- `apply`: `CommentPosted{n, Ok}` → notice `Comment posted on #n`, and `[LoadConversation{n}]` if `opened == Some(n)`; `Err` → error notice. `ReviewSubmitted{n, Ok}` → clear `pending_comments`, rebuild rows, notice `Review submitted for #n`, and `[LoadConversation{n}, LoadPullRequests]` (recorded in flight); `Err` → error notice, pending kept.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn comment_on_diff_line_opens_line_compose()      // target LineComment{path "a.rs", line 21, side Right, excerpt "+c"}
#[test] fn comment_on_hunk_header_is_refused()           // notice text, mode Browse
#[test] fn comment_without_open_pr_is_refused()
#[test] fn comment_elsewhere_opens_pr_compose()          // focus Files → PullRequestComment
#[test] fn typing_edits_compose_text()                   // Type('h'),Type('i'),Newline,Backspace → "hi"
#[test] fn cancel_discards_compose()
#[test] fn submitting_line_comment_adds_pending_and_rows() // pending len 1, a Comment row with header "pending" exists, notice "Pending comments: 1"
#[test] fn submitting_empty_comment_is_refused()
#[test] fn submitting_pr_comment_posts_and_reloads_on_success() // → [PostComment{..}]; apply CommentPosted Ok → [LoadConversation{2}], notice contains "posted"
#[test] fn review_flow_picks_event_then_submits_with_pending() // Review → Pick(Approve) → Type.. → Submit → [SubmitReview{review.event Approve, comments len 1, commit_sha "sha2"}]
#[test] fn request_changes_requires_body()
#[test] fn comment_review_needs_body_or_pending()
#[test] fn successful_review_clears_pending_and_reloads()   // ReviewSubmitted Ok → pending empty, [LoadConversation{2}, LoadPullRequests]
#[test] fn failed_review_keeps_pending_comments()           // Err → pending len 1, notice error
#[test] fn browse_keys_are_ignored_while_composing()        // Down in Compose → pr_index unchanged, returns []
```

- [ ] **Step 2: Run `cargo test app::`** — Expected: compile failure or assertion failures.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run `cargo test`** — Expected: all pass.
- [ ] **Step 5: Commit** — `git add -A && git commit -m "feat: compose PR comments, pending line comments, and review submissions"`

---

### Task 7: Mode-aware keymap

**Files:**
- Modify: `src/keymap.rs`

**Interfaces:**
- Produces: `pub fn action_for(key: KeyEvent, mode: &Mode) -> Option<Action>`.
- `Compose`: `Esc` → `Cancel`, `Ctrl-s` → `Submit`, `Enter` → `Newline`, `Backspace` → `Backspace`, `Char(c)` without Ctrl → `Type(c)`, else `None`.
- `PickReviewEvent`: `a` → `Pick(Approve)`, `r` → `Pick(RequestChanges)`, `c` → `Pick(Comment)`, `Esc` → `Cancel`, else `None`.
- `Browse`: v1 bindings plus `Esc` → `Escape`, `v` → `ToggleTab`, `c` → `Comment`, `R` → `Review`.

- [ ] **Step 1: Write the failing tests** (`maps_compose_keys`, `maps_pick_keys`, `maps_new_browse_keys`; update the v1 tests to pass `&Mode::Browse`).
- [ ] **Step 2: Run `cargo test keymap::`** — Expected: compile failure.
- [ ] **Step 3: Implement.** `main.rs` passes `&app.mode`.
- [ ] **Step 4: Run `cargo test`** — Expected: all pass.
- [ ] **Step 5: Commit** — `git add -A && git commit -m "feat: mode-aware key bindings"`

---

### Task 8: UI: cards, tabs, conversation pane, status bar

**Files:**
- Modify: `src/ui.rs`

**Interfaces:**
- Produces: `pub fn render(frame: &mut Frame, app: &App) -> Viewport`. Layout: outer `[Min(0), Length(2)]`; main `[Min(32), Percentage(65)]`; right `[Length(1), Min(0)]` (tab bar, content). Tab bar text ` Changes │ Conversation ` with the active tab bold yellow. Content by `app.tab`: Changes = files list over diff (diff drawn by Task 9; this task draws rows as plain text); Conversation = `conversation_rows(..)` from `conversation_scroll`, styled by kind (Title bold, Meta dim, Heading cyan, Author bold, Thread yellow), block title `Conversation · #N`, or `Open a pull request` when none.
- Cards: three lines per PR plus a blank line: `#N title`; `  author · head → base`; `  ` then markers: checks `✓`/`✗`/`●` with word `checks`, review `✓ approved`/`✗ changes`/`○ review`, `draft`, `N 💬` when N > 0, separated by two spaces; selected card `Style::new().bg(Color::DarkGray)`.
- Status bar row 1: left context: `Pull requests` / `#N · files` / `#N · in {filename}:{line}` (line from `commentable_at_cursor`, else `#N · in {filename}`) / `#N · conversation` / `Composing…` / `Pick a verdict`, then ` · N pending` when pending comments exist; right: `busy_message()` yellow, else `notice` (red when error, green otherwise). An error notice takes the whole row. Row 2 hints by mode and focus (exact strings are the builder's; each must mention `Esc` and `q` in Browse, `Ctrl-s` in Compose, `a`/`r`/`c` in Pick).
- Returned `Viewport`: inner sizes of the conversation and diff blocks (when a block is not drawn this frame, keep the other side's width and the right content height).

- [ ] **Step 1: Write the failing tests** (helpers gain the new fields)

```rust
#[test] fn cards_show_status_markers()                  // Success+Approved+draft+2 comments → "✓ checks", "✓ approved", "draft", "2"
#[test] fn card_without_checks_or_reviews_has_no_markers() // None/None → no "✓", no "✗", no "○"
#[test] fn tab_bar_marks_active_tab()
#[test] fn conversation_tab_renders_body_and_comments()
#[test] fn status_bar_shows_file_and_line_when_in_diff()  // "#1 · in src/a.rs:21"
#[test] fn status_bar_counts_pending_comments()           // "1 pending"
#[test] fn render_returns_viewport_sizes()                // 100x30 → diff_height > 0 and conversation_width > 0 after switching tabs
#[test] fn renders_conversation_in_tiny_terminal()        // 20x5, 500-char body line → no panic
```

- [ ] **Step 2: Run `cargo test ui::`** — Expected: compile failure.
- [ ] **Step 3: Implement.** `main.rs` stores the returned viewport (`app.set_viewport`).
- [ ] **Step 4: Run `cargo test`** — Expected: all pass.
- [ ] **Step 5: Commit** — `git add -A && git commit -m "feat: sidebar cards, tab bar, conversation pane, and status bar"`

---

### Task 9: UI: diff rows, cursor, overlays

**Files:**
- Modify: `src/ui.rs`

**Interfaces:**
- Diff pane draws `diff_rows[diff_offset..]` for the block height: `Line` rows coloured as v1; `Comment` rows indented four spaces, `header` on its own dim line, magenta text for existing comments, yellow text with the `pending` header for pending; the cursor row gets `Style::new().reversed()` when focus is `Diff`. Tabs in `Line` text are expanded to four spaces here (moved from `parse_patch`).
- Overlays drawn last with `Clear`: `Compose` → centered box 70% wide, 50% tall, title by target (`Comment on #N`, `Comment on {path}:{line}`, `Approve #N`, `Request changes on #N`, `Comment review on #N`), text rows plus `▏` after the last character, bottom hint `Ctrl-s send · Esc cancel · Enter newline`; for a `LineComment` the excerpt is the first row, dim. `PickReviewEvent` → centered 5-row box `Review #N: (a) approve  (r) request changes  (c) comment  Esc cancel`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test] fn diff_shows_inline_comment_rows()        // "ann · 2026-01-02" and comment text appear below "+c"
#[test] fn compose_overlay_shows_title_and_text()  // "Comment on src/a.rs:21", "hello▏"
#[test] fn pick_overlay_lists_verdicts()           // "(a) approve"
#[test] fn parse_patch_keeps_tabs_and_ui_expands() // in diff.rs: text keeps "\t"; in ui: rendered row has four spaces
```

- [ ] **Step 2: Run `cargo test`** — Expected: compile failure / assertion failures (the diff.rs tab test flips: update `expands_tabs_to_four_spaces` to `keeps_tabs_verbatim`).
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run `cargo test`** — Expected: all pass.
- [ ] **Step 5: Commit** — `git add -A && git commit -m "feat: diff cursor, inline comment rows, compose and verdict overlays"`

---

### Task 10: Wiring, README, verification

**Files:**
- Modify: `src/main.rs`, `README.md`

- [ ] **Step 1: Finish `main.rs`**: `start` dispatches `Action::Refresh` and sends its requests; the loop captures the `Viewport` from `render` and calls `set_viewport`; `action_for(key, &app.mode)`; every `Vec<Request>` from `dispatch` and `apply` is sent.
- [ ] **Step 2: `cargo clippy --all-targets -- -D warnings` and `cargo test`** — Expected: clean, all pass. `cargo test -- --ignored live_` — Expected: both live reads pass.
- [ ] **Step 3: Live drive (reads only)** with the `expect` driver: PR list shows cards with markers; `Enter` loads files and conversation; `v` shows the conversation with the body; `v` back, `l` into the diff, `j` moves the cursor and the status bar shows `in <file>:<line>`; `c` opens the compose box, type two words, `Esc` cancels; `R` shows the verdict box, `Esc` cancels; `Esc` twice returns to the PR list; `q` exits 0. Nothing is sent.
- [ ] **Step 4: README**: new key table from the spec, a paragraph on reviewing (pending comments go with the review; PR comments post at once), and the GraphQL note.
- [ ] **Step 5: Commit** — `git add -A && git commit -m "feat: wire review features and document them"`
