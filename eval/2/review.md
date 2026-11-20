# Review of run 2
## Summary
Run 2 is a file split of `src/state.rs`, done in 13 move commits and 1 comment fix. The file goes from 4,877 to 1,722 lines. Eleven new modules hold code that was scattered through the file: `batch`, `clock`, `message`, `message_move_task`, `pagination`, `queue`, `queue_address`, `queue_attributes`, `queue_creation`, `receipt_handle` and `tags`. One more commit moves the attribute digest and size helpers into the existing `state/message_attributes.rs`. Every commit is a verifiable pure move: per-commit line multisets match exactly, apart from new module headers, `mod`/`pub use` lines, two removed section banners, and five items raised from private to `pub(super)`. Nothing else was touched. The attempt leaves alone the string-tuple queue keys, the string attribute map re-parsed on read, the `#[allow]`s, the narration comments, the giant handlers in `ops/`, the clones and the tests. Metrics and clippy output are the same as baseline, apart from the shift caused by the new files.

## Commit classification
| hash | subject | kind | pure move? |
|---|---|---|---|
| 221f61902 | move attribute digest and size helpers next to the attribute rules | move | yes |
| 27e2f77e4 | extract queue key, url and ARN handling into state::queue_address | move | yes (1 fn to pub(super)) |
| 3670b8c18 | extract MessageMoveTask and its limits into state::message_move_task | move | yes (3 fns to pub(super), banner dropped) |
| 5e1c6de3d | extract receipt handle encoding and refusals into state::receipt_handle | move | yes (fn + const to pub(super)) |
| 874c93698 | extract attribute validation into state::queue_attributes | move | yes |
| 33cd4743f | move the attribute catalog next to the code that validates against it | move | yes |
| a3af63097 | extract tag limits and refusals into state::tags | move | yes |
| f3d303b17 | extract create-time name and recreate checks into state::queue_creation | move | yes |
| cfc0e1f96 | extract list pagination helpers into state::pagination | move | yes |
| 46fafaef5 | point the tag-bounds note at the module that now holds them | other (comment) | no |
| a45b6a8fd | extract the stored message records into state::message | move | yes |
| 479bfc304 | move Queue and its impl into state::queue | move | yes (banner dropped) |
| c88aca4da | extract epoch rendering and span arithmetic into state::clock | move | yes (1 fn to pub(super)) |
| ca7b38e07 | extract batch limits and entry wording into state::batch | move | yes |

## Scores
| dimension | score | evidence |
|---|---|---|
| 1 Structure | +2 | state.rs 4877 -> 1722 lines. The concerns are named modules declared at state.rs:180-216 (e.g. 479bfc304 state/queue.rs, 27e2f77e4 state/queue_address.rs:14). There is no `pub(crate)` creep: only 7 items go to `pub(super)`, e.g. message_move_task.rs:195. The baseline glob pattern is kept: `use super::*` plus `pub use x::*`. |
| 2 Duplication | 0 | No helper was merged or removed. Every commit only relocates code. |
| 3 Error handling | 0 | Untouched. `after` still has its unreachable `unwrap_or(at)` (clock.rs:46), and `integer()` keeps its dead default (queue.rs:281). |
| 4 Domain types | 0 | `queues: BTreeMap<(String, String), Queue>` is unchanged (state.rs:577), as are `handles: HashMap<String, String>` (queue.rs:47) and `attributes: BTreeMap<String, String>` (queue.rs:30). |
| 5 Ownership | 0 | `queue_key` still allocates two Strings per lookup (queue_address.rs:14-16). |
| 6 Comments | 0 | Two banners were removed (3670b8c18, 479bfc304), and one comment the move would have left stale was fixed (46fafaef5, state.rs:520). Each new module adds a `///` line on its `mod` plus a 2-line `//!` header. Comment share stays at 43.4% (metrics.txt). |
| 7 Control flow | 0 | No function body changed. |
| 8 Lint suppressions | 0 | clippy.txt is identical to baseline (196 allow_attributes_without_reason, 142 allow_attributes). |
| 9 Tests | 0 | No test file is in the diffstat. 280/280 pass as in baseline. |
| 10 Commit hygiene | +2 | One concern per commit. Each message says why and states "pure move" plus any visibility change (e.g. 5e1c6de3d, 3670b8c18). Each commit can be checked mechanically. Minor nit: 46fafaef5 could have been folded into a3af63097. |
| 11 Behavior risk | 0 | No code bodies changed. Visibility changes only widen to `pub(super)`. The glob re-exports compile without ambiguity and tests are green. |

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
| S-18 | untouched | |
| S-19 | untouched | |
| S-20 | untouched | |
| S-21 | untouched | |

## Best hunks
479bfc304 (and siblings), state.rs:195-204. These lines are the module map that replaced roughly 3,100 inline lines; each module is named for the concern it holds:
```rust
/// A queue, the messages it holds and what it does with them.
mod queue;
/// The key, url and ARN a queue is addressed by, and the reading of each back into a name.
mod queue_address;
/// How a queue's attributes are validated and stored, and what refuses each way they can be wrong.
mod queue_attributes;
/// The name a queue may be created under, and when a create of an existing name is a repeat.
mod queue_creation;
/// What a receipt handle is, and the refusals one that cannot be resolved is answered with.
mod receipt_handle;
```
Why: a reader can now find where a rule lives from the module list instead of searching a 4.9k-line file.

46fafaef5, state.rs:520:
```rust
-// The per-queue tag bounds live with the other tag quotas above:
+// The per-queue tag bounds live with the other tag rules in `tags`:
```
Why: the attempt noticed that its own move made a comment wrong and fixed it in a separate commit with an explicit reason.

## Worst hunks
27e2f77e4, state/queue_address.rs:6 (the same line appears in every new module):
```rust
use super::*;
```
Why: each new module glob-imports the whole parent and is glob re-exported back (`pub use queue_address::*`), so the split hides each module's real dependencies. It follows the baseline pattern but adds 11 more instances, and the glob-use rate rises from 4.03 to 5.13 per KLOC (metrics.txt).

## Silent edits check
Every commit was compared mechanically: the multiset of removed lines (trimmed) against the multiset of added lines, per commit. The only differences are module headers, `mod`/`pub use` lines, two `// ----` banners plus their titles, and `pub(super)` on `queue_arn_fields`, `is_of`, `in_progress`, `deliverable_by`, `encoded_opaque_token`, `RECEIPT_HANDLE_FIELD_SEPARATOR` and `epoch_seconds`. I also compared whole bodies (brace-extracted and hashed) against baseline `state.rs`. These were identical: `queue_key` (3385), `Queue::integer` (1440), `validate_attributes` (4210), `queue_name_refusal` (4393), `recreate_refusal` (4526), `Queue::materialise_due` (1764), `after` (4658), `is_fifo` (1348) and `struct StoredMessage` (963). `encoded_opaque_token` and `deliverable_by` differ only by the `pub(super)` keyword. I found no silent edits.
