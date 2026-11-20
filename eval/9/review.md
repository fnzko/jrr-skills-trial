# Review of run 9
## Summary
Run 9 is a pure file split of the SQS `state.rs` (4,877 lines at baseline) into 13 new
submodules under `state/` (tags, queue_address, receipt_handle, message_move_task, refusals,
queue_attributes, queue, redrive members, pagination, batch, limits, clock, store, message_rules),
leaving `state.rs` at 355 lines of module docs, `mod`/`pub use *` declarations and a few constants.
Every commit says "Items are moved verbatim", and a sorted-line comparison of all state files
confirms it: the only differences are imports, 11 `private -> pub(super)` widenings, new
SPDX/module doc headers and rustfmt reflows. Nothing else was touched: no type, error-handling,
ownership, control-flow, lint or test change, and clippy output is identical to baseline. The moves
leave several section banners and cross-references orphaned, and one new module doc contradicts the
code in its own file.

## Commit classification
| hash | subject | kind | pure move? |
|---|---|---|---|
| 0631780e1 | move queue tag rules out of state.rs into state/tags.rs | move | yes |
| 260cce00d | move queue addressing into state/queue_address.rs | move | yes |
| db5b31815 | move receipt handle rules into state/receipt_handle.rs | move | yes |
| 8ac3dc493 | move the message move task record into state/message_move_task.rs | move | yes |
| 44d37a4d3 | gather refusal wording and error constructors in state/refusals.rs | move | yes |
| 88eac22cc | move queue attribute catalogue and validation into state/queue_attributes.rs | move | yes |
| d074541da | move Queue and its message records into state/queue.rs | move | yes |
| b1be74695 | move the redrive attribute member names next to the redrive validation | move | yes (2 consts -> pub(super)) |
| 188d1b86e | move leftover name and attribute helpers to the modules that own them | move | yes (list_member -> pub(super)) |
| 2d5c4a756 | move listing pagination into state/pagination.rs | move | yes |
| 2819c4449 | move the shared batch rules into state/batch.rs | move | yes |
| 21642cdc1 | collect the store bounds in state/limits.rs | move | yes |
| 2c1af8d50 | move clock and timestamp helpers into state/clock.rs | move | yes (2 fns -> pub(super)) |
| 4849caed5 | move the queue store into state/store.rs | move | yes |
| 0dbe5f9fd | move message body and FIFO token rules into state/message_rules.rs | move | yes |

## Scores
| dimension | score | evidence |
|---|---|---|
| 1. Structure | +1 | state.rs 4877 -> 355 lines, cohesive files (4849caed5 state/store.rs, 21642cdc1 state/limits.rs); only `pub(super)`, no `pub(crate)`. Held back by 13 new `pub use X::*` globs (state.rs:219-236) that keep one flat namespace, and `impl From<&MessageAttributeValue> for AttributeParts` left in state.rs:327-353 while the struct lives in state/message_attributes.rs:404 |
| 2. Duplication | 0 | no code removed, only relocated |
| 3. Error handling | 0 | unchanged |
| 4. Domain types | 0 | `(String, String)` keys untouched, e.g. state/message_move_task.rs:162 `is_of(&self, key: &(String, String))` |
| 5. Ownership | 0 | metrics clone()/to_string() rates unchanged (only LOC grew) |
| 6. Comments | -1 | orphaned banners: state/clock.rs:13-15 "Message movement tasks" above `epoch_seconds` (2c1af8d50); state/limits.rs:68-70 "Messages" banner at EOF; state/batch.rs:56-68 "Parameter-level refusals" block at EOF; state/pagination.rs:107 "tag quotas above" now in tags.rs. New doc state/limits.rs:3 contradicts limits.rs:17,59 (21642cdc1). Comment share 43.4% -> 43.2% |
| 7. Control flow | 0 | no function bodies changed |
| 8. Lint suppressions | 0 | clippy.txt identical to baseline; #[allow( count unchanged |
| 9. Tests | 0 | no test files changed |
| 10. Commit hygiene | +2 | 15 single-purpose move commits, each body states why and notes visibility widenings (e.g. 2c1af8d50, 188d1b86e); reviewable in order with --color-moved |
| 11. Behavior risk | 0 | sorted-line diff of all state files shows only imports, visibility, docs and fmt reflows |

## Audit findings
| id | status | evidence |
|---|---|---|
| S-01 | untouched | |
| S-05 | untouched | |
| S-07 | untouched | |
| S-08 | untouched | |
| S-09 | untouched | |
| S-12/S-13 | untouched | |
| S-14/S-15 | untouched | |
| S-16/S-17 | untouched | |
| S-18 | untouched | files split but no function shortened, state struct unchanged (audit says file size was not the issue) |
| S-19 | untouched | narration kept verbatim, e.g. state/queue.rs:348 "Nothing has been sequenced yet" |
| S-20 | untouched | no contradiction fixed; one new one added (limits.rs:3) |
| S-21 | untouched | |

## Best hunks
4849caed5, state/store.rs:1-20 (with state.rs:207-209)
```rust
//! The store: every queue of every region, and the operations that resolve, move and expire them.
...
/// The store: every queue of every region, and the operations that resolve, move and expire them.
mod store;
```
The ~800-line store impl gets its own file, so the per-queue record (queue.rs) and the store read separately.

21642cdc1, state/limits.rs:5-17
```rust
/// How many queues this emulator will hold; `CreateQueue` refuses past it.
pub const MAX_QUEUES: usize = 1_000;

/// How many recently-deleted queue names the store remembers (see
/// [`ComAmazonawsSqsAmazonsqsState::recently_deleted`]).
pub const MAX_RECENTLY_DELETED_NAMES: usize = MAX_QUEUES;
```
All store caps previously scattered through state.rs are now in one place for a memory review.

## Worst hunks
21642cdc1, state/limits.rs:3 vs 50-59
```rust
//! The bounds that keep the in-memory store finite. A bound is refused, never enforced by eviction.
...
/// ... which is why this bound is enforced by eviction and the message
/// bounds are not.
pub const MAX_DEDUPLICATION_TOKENS_PER_QUEUE: usize = 1_000;
```
The new module doc states a rule that three constants in the same file explicitly break.

2c1af8d50, state/clock.rs:13-19
```rust
// ---------------------------------------------------------------------------
// Message movement tasks
// ---------------------------------------------------------------------------

/// A `SystemTime` as the whole seconds since the Unix epoch, which is how SQS reports
pub(super) fn epoch_seconds(at: SystemTime) -> String {
```
A banner dragged along by the move now labels clock helpers as "Message movement tasks".

## Silent edits check
Compared every non-blank line of baseline `state.rs` + `state/*.rs` against HEAD (sorted,
whitespace-trimmed): 228 differing lines, all imports, `mod`/`pub use` lines, SPDX and module
docs, 11 `pub(super)` additions and rustfmt reflows of identical expressions
(`designates_the_fifo_kind`, `compared` BTreeSet, `nanos`, `retained_message_bytes` sum,
`runs.push(...)`, `ask_handle`/`destination_arn` in the task listing, deduplication `retain`).
Spot-checked `deliverable_by` and `is_of` (message_move_task.rs:162,197), `queue_arn_fields`
(queue_address.rs:143), `list_member` (message_attributes.rs:416), `epoch_seconds` and
`count_as_i64` (clock.rs): bodies identical to baseline, only visibility changed. No silent edits.
