# Review of run 10
## Summary
The attempt made five small commits. The first splits the 4.9k-line `state.rs` into eight
sibling modules (`state/{catalogue,queue,store,message_move,addressing,refusals,parameters,content}.rs`)
with glob re-exports, so `state.rs` drops to 248 lines. Three commits change code: one
`authorization::denied_for_queue(&Queue)` helper replaces the ARN/policy copy-and-pass block in
18 handlers, TagQueue/UntagQueue/AddPermission/RemovePermission resolve the url to the queue in
one lookup instead of filter + expect, and message-move status becomes an enum. The last commit
drops the section banners the split made redundant. It leaves alone the stringly identifiers and
`(String, String)` keys, the attribute string map, the per-op `lift`/`store_unavailable`
scaffolding and `#[allow]`s, the giant handlers, the comment volume, and every performance finding.

## Commit classification
| hash | subject | kind | pure move? |
|---|---|---|---|
| 9b7c967f6 | refactor: split state.rs into one module per concern | move | yes (only `pub(super)` on 10 items, module docs, `use super::*`) |
| 71c27d34a | refactor: authorize a queue through one helper instead of copying its fields | dedupe | no |
| 57a901818 | refactor: model a message movement task's status as an enum | rewrite | no |
| 4738b88d2 | refactor: resolve a queue url to its queue in one lookup | rewrite | no |
| 484424bcb | docs: drop section banners now that each section is its own module | other (comments) | no |

## Scores
| dimension | score | evidence |
|---|---|---|
| Structure | +1 | 9b7c967f6: `state.rs` 4.9k -> 248 lines, concerns in `state/store.rs` (821), `state/queue.rs` (1355) etc.; minus: every new module starts with `use super::*` (`state/store.rs:5`) and is re-exported by glob (`state.rs:208-215`), glob-use metric 4.03 -> 4.86 |
| Duplication | +1 | 71c27d34a: 18 copies of the `(queue.arn.clone(), queue.policy().map(str::to_string))` + 5-arg `denied` call collapsed into `denied_for_queue` (`state/authorization.rs:355`); per-op `lift`/`store_unavailable` untouched |
| Error handling | +1 | 71c27d34a + 4738b88d2: `expect("the queue was resolved out of this same store")` 12 -> 3; e.g. `ops/tag_queue.rs:192-196` now `queue_by_url_mut(..).ok_or_else(..)?` |
| Domain types | +1 | 57a901818: `MessageMoveStatus` enum with `as_str`/`Display` (`state/message_move.rs:67-99`) replaces four `&str` constants compared as strings; no identifier newtypes |
| Ownership | +1 | 71c27d34a: 18 `arn.clone()` + 18 `policy().map(str::to_string)` removed (0 left); 4738b88d2 `state/store.rs:127-129` no longer allocates the name; clone() 8.42 -> 7.53 /kloc; `queue_key` still allocates |
| Comments | 0 | 484424bcb removes 5 banners (banner metric 0.61 -> 0.10) but 8 new module docs added; comment share 43.4% -> 43.6% |
| Control flow | 0 | handlers lose a few lines (e.g. `ops/untag_queue.rs:189-201`), no handler split, no let-else introduced |
| Lint suppressions | 0 | `clippy.txt` identical to baseline (360 total) |
| Tests | 0 | no test file changed |
| Commit hygiene | +2 | five commits, one kind each, bodies state why (e.g. 71c27d34a names the 18 handlers and the six double lookups); move isolated from edits |
| Behavior risk | 0 | checked `queue_by_url_mut` == old `queue_name_from_url().filter(holds_queue)` path (`state/store.rs:127`), auth now on `&mut` ref of same queue (`ops/set_queue_attributes.rs:333-340`), `source.arn` replacing copied `queue_arn` refers to the same source queue (`ops/start_message_move_task.rs:394,433`); status wire text unchanged |

## Audit findings
| id | status | evidence |
|---|---|---|
| S-01 | untouched | `queue_key` still `(String, String)` at `state/addressing.rs:42` |
| S-05 | untouched | attribute map and `integer()` moved verbatim |
| S-07 | untouched | |
| S-08 | untouched | |
| S-09 | untouched | |
| S-12/S-13 | partial | 71c27d34a/4738b88d2: untag_queue exemplar now one lookup (`ops/untag_queue.rs:192`), expects 12 -> 3, 18 policy copies gone; `queue_key` still allocates two Strings per lookup |
| S-14/S-15 | untouched | |
| S-16/S-17 | untouched | |
| S-18 | untouched | file split only; `receive_message.rs` handler untouched in length |
| S-19 | untouched | banners removed but comment share rose slightly |
| S-20 | untouched | |
| S-21 | untouched | (STS, outside this crate) |

## Best hunks
71c27d34a + 4738b88d2, `ops/tag_queue.rs:192-201` (same shape in untag, add/remove permission):
```rust
        let queue = store
            .queue_by_url_mut(&region, &input.queue_url)
            .ok_or_else(|| {
                TagQueueErrorModeled::QueueDoesNotExist(state::queue_does_not_exist())
            })?;
        if let Some(denied) =
            state::authorization::denied_for_queue(&*self.engine, ctx, Self::NAME, queue)
        {
            return Err(denied);
        }
```
Replaces a filter/copy/lookup/expect/clone/to_string sequence of ~25 lines with one lookup and a borrowed authorization call.

57a901818, `state/message_move.rs:66-88`:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
            Self::Completed => "COMPLETED",
```
A closed set compared by variant instead of string, wire text in one place.

## Worst hunks
9b7c967f6, `state/store.rs:5` (and every new sibling) with `state.rs:208-215`:
```rust
use super::*;
...
pub use catalogue::*;
pub use queue::*;
pub use store::*;
```
The split keeps everything in one flat namespace via globs, so the new files do not show their dependencies and the module boundaries are cosmetic.

## Silent edits check
Compared the full line multiset of baseline `state.rs` against HEAD of the eight new files plus
`state.rs` at 9b7c967f6: the only differing lines are 10 items made `pub(super)`
(`deliverable_by`, `in_progress`, `is_of`, four constants, `encoded_opaque_token`,
`epoch_seconds`, `queue_arn_fields`), the 8 module docs, SPDX headers, `mod`/`pub use` lines
and `use super::*`. No silent body edits. Later commits touch moved code only in the hunks named
above (`queue_by_url_mut`, `message_move_tasks` status comparisons in `state/store.rs:503,789`).
