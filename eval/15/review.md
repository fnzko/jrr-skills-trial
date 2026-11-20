# Review of run 15
## Summary
A small, targeted attempt of 11 commits (318+/434-). It removes every hand-written `#[allow]` outside the generated `ops/types` files, deletes two `override_*` methods that restate the vendored trait defaults, and removes the lookup-then-`expect` pattern from six handlers by resolving the queue once and authorising against borrowed fields (11 of 14 `expect(` in src gone, 6 of 18 `policy().map(str::to_string)` copies gone). It introduces one domain enum (`MessageMoveStatus`) in place of four `&'static str` constants and moves the message-move-task section out of `state.rs` into `state/message_move.rs` verbatim. It leaves alone the stringly state (`(String, String)` keys, attribute string map), the per-request sweeps and the long-poll loop, the per-op `lift`/`store_unavailable` boilerplate, the giant handlers and the comment volume.

## Commit classification
| hash | subject | kind | pure move? |
|---|---|---|---|
| 6c0b4d5ae | drop unused_imports allows that hide nothing | other (lint) | no |
| 677e4da5a | drop lint allows on the types re-exports | other (lint) | no |
| 1db883274 | drop crate-level lint allows that hide nothing | other (lint) | no |
| 8deb70ad9 | remove override_* methods that restate the trait defaults | dedupe | no |
| 3ead399fc | resolve the queue once in tag/untag/add/remove permission ops | rewrite | no |
| f495a9406 | authorise SetQueueAttributes against borrowed queue fields | rewrite | no |
| c3540965e | model message move task status as an enum | rewrite | no |
| d33f362ab | rename scratch_probe to say what it covers | other (test rename) | yes (100% rename) |
| 397e338b3 | drop the remaining lookup-then-expect in move-task and set-attributes | rewrite | no |
| 18e5d8e1e | move message move task types out of state.rs into state/message_move.rs | move | yes (plus 4 `pub(super)`) |
| 2e594ec46 | drop a needless borrow of the queue name | other (clippy fix-up) | no |

## Scores
| dimension | score | evidence |
|---|---|---|
| Structure | +1 | 18e5d8e1e: cohesive `state/message_move.rs` (275 lines), `state.rs` 4877 -> 4628, re-exported at state.rs:193-198; only 4 helpers widened to `pub(super)` (message_move.rs). Small dent: `use super::*;` at message_move.rs:8 |
| Duplication | +1 | 8deb70ad9 deletes the two no-op overrides from service.rs (37 lines); 3ead399fc removes the triple lookup repeated in 4 handlers (add_permission.rs:230, remove_permission.rs:190, tag_queue.rs:188, untag_queue.rs:188). `lift`/`store_unavailable` copies untouched |
| Error handling | +1 | 3ead399fc, 397e338b3: `expect("the queue was resolved out of this same store")` 12 -> 1 (set_queue_attributes.rs:329 kept, justified in message); typed errors via let-else at start_message_move_task.rs:281 and `and_then` at set_queue_attributes.rs:240-241 |
| Domain types | +1 | c3540965e: `MessageMoveStatus` enum with `as_str`/`Display` (message_move.rs:70-95), `status()` returns it (message_move.rs:255) |
| Ownership | +1 | 3ead399fc/f495a9406: `queue.arn.clone()` and `policy().map(str::to_string)` dropped in 6 handlers (e.g. tag_queue.rs:200-201); `queue_by_url_mut` no longer allocates the name (state.rs:2405-2406); name kept as `&str` in set_queue_attributes.rs |
| Comments | 0 | comment share 43.4% -> 43.6%; only the override narration in service.rs removed and enum variant docs added; no narration pass |
| Control flow | +1 | let-else at start_message_move_task.rs:281; handlers lose the scoped-borrow blocks (add_permission.rs:230-246); no giant handler split |
| Lint suppressions | +2 | 6c0b4d5ae, 677e4da5a, 1db883274: `#[allow(` 3.78 -> 1.23/KLOC, clippy allow findings 360 -> 166; remaining 24 only in generated `ops/types/*_types.rs` |
| Tests | 0 | d33f362ab renames tests/all/scratch_probe.rs -> delete_message_batch_survivors.rs (tests/all.rs:124-125), content identical |
| Commit hygiene | +1 | one kind per commit, messages explain why; but 2e594ec46 and 397e338b3 are fix-ups of f495a9406 (same file, same hunk area) that could have been folded |
| Behavior risk | 0 | override defaults confirmed identical to vendor/smithy-server-runtime/src/service.rs:390-411; status strings unchanged via `as_str`; authorization still runs under the same store guard as before (3ead399fc) |

## Audit findings
| id | status | evidence |
|---|---|---|
| S-01 | untouched | |
| S-05 | untouched | |
| S-07 | untouched | |
| S-08 | untouched | |
| S-09 | untouched | |
| S-12/S-13 | partial | 3ead399fc: untag_queue.rs exemplar now resolves once (untag_queue.rs:188-202); 11 of 12 queue expects and 6 of 18 policy copies removed; `queue_key` still allocates per lookup |
| S-14/S-15 | untouched | |
| S-16/S-17 | partial | 6c0b4d5ae/677e4da5a remove all `#[allow(unused_imports)]` on op globs and type re-exports; `lift`/`store_unavailable` duplicates and glob imports remain |
| S-18 | untouched | |
| S-19 | untouched | |
| S-20 | untouched | |
| S-21 | untouched | |

## Best hunks
3ead399fc, ops/untag_queue.rs:188-202 (same shape in 3 siblings):
```rust
        let queue = store
            .queue_by_url_mut(&region, &input.queue_url)
            .ok_or_else(|| {
                UntagQueueErrorModeled::QueueDoesNotExist(state::queue_does_not_exist())
            })?;
        if let Some(denied) = state::authorization::denied(
            &*self.engine,
            ctx,
            Self::NAME,
            &queue.arn,
            queue.policy(),
        ) {
```
One lookup replaces three, removing two clones and two `expect` panics exactly as the audit prescribed.

c3540965e, state/message_move.rs:70-90:
```rust
pub enum MessageMoveStatus {
    Running,
    Completed,
    Cancelling,
    Cancelled,
}
impl MessageMoveStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "RUNNING",
            ...
```
A closed set becomes a type, with the wire spelling in one place and comparisons checked by the compiler.

## Worst hunks
397e338b3, ops/set_queue_attributes.rs:240-241:
```rust
        let (name, fifo) = state::queue_name_from_url(&input.queue_url)
            .and_then(|name| Some((name, store.queue(&region, name)?.is_fifo())))
```
Correct, but a `?` inside a closure building a tuple reads worse than a let-else, and the handler still looks the queue up again with an `expect` at :329.

18e5d8e1e, state/message_move.rs:8:
```rust
use super::*;
```
The extracted module pulls in the whole parent namespace, so its real dependencies stay invisible.

## Silent edits check
18e5d8e1e: diffed every removed line of state.rs against the new state/message_move.rs. Only differences: the banner comment replaced by an SPDX header, `//!` doc and imports, and `pub(super)` added to `is_of`, `in_progress`, `deliverable_by`, `rate`. Bodies of `MessageMoveTask::status`, the bounds constants and `MessageMoveStatus` are byte-identical to the previous commit. d33f362ab is a 100% similarity rename. No silent edits found.
