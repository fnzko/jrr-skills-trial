# Review of run 6
## Summary
The attempt splits the 4,877-line `src/state.rs` into nine concern modules under `src/state/` (queue_address, receipt_handle, clock, message_digest, message_move_task, queue_tags, queue_attributes, queue, store), leaving `state.rs` at 1,241 lines of constants, wording helpers and module wiring. Nine of eleven commits are verified pure moves; the only edits inside them are `pub(super)` on items the sibling modules now need (same effective scope as the old private items) and re-wrapped module docs. One real code change adds `Queue::authorization_subject` and replaces 18 identical `(queue.arn.clone(), queue.policy().map(str::to_string))` spellings in the ops. A final commit fixes the README layout section. It left everything else alone: string-keyed state and attribute maps, `expect` after prior lookups, scaffold boilerplate and `#[allow]`s, giant handlers, comment volume, and all performance findings.

## Commit classification
| hash | subject | kind | pure move? |
|---|---|---|---|
| 9b7a26dc7 | move queue address helpers out of state.rs | move | yes (+1 `pub(super)`) |
| 45e82ef7c | move receipt handle format and refusals out of state.rs | move | yes (+2 `pub(super)`) |
| 76625f120 | move instant and interval helpers out of state.rs | move | yes (+1 `pub(super)`) |
| e7f48a2d0 | move message identifiers and digests out of state.rs | move | yes |
| 15e1892af | move message movement task record and wording out of state.rs | move | yes (+3 `pub(super)`, banner dropped, earlier module docs re-wrapped) |
| 2fd61bc47 | move queue tag limits and refusals out of state.rs | move | yes |
| c225704a7 | move the settable queue attribute schema out of state.rs | move | yes |
| bd3b99d23 | move the queue record and its message plane out of state.rs | move | yes |
| d39e3f236 | move the store struct and its operations out of state.rs | move | yes |
| 4e876f7e4 | name the ARN and policy pair every authorization check reads | dedupe | no |
| 86fd13b2d | describe the state module layout in the README | other (docs) | no |

## Scores
| dimension | score | evidence |
|---|---|---|
| Structure | +2 | state.rs 4877 -> 1241 lines; cohesive modules e.g. `state/queue_address.rs` (173), `state/receipt_handle.rs` (225), `state/store.rs` (823) wired at state.rs:199-234 (commits 9b7a26dc7..d39e3f236). No pub creep: new `pub(super)` (e.g. queue_address.rs:135, message_move_task.rs:159-195) matches the old private scope. Minus: each module uses `use super::*` (queue_address.rs:6) and `pub use x::*`; queue.rs is still 1362 lines. |
| Duplication | +1 | 4e876f7e4: 18 handlers now call `queue.authorization_subject()` (state/queue.rs:391-395; ops/delete_message.rs:212, ops/add_permission.rs:244). Narrow, but real removal, not relocation of whole helpers. |
| Error handling | 0 | `expect("the queue was resolved out of this same store")` unchanged (ops/add_permission.rs:243,257). |
| Domain types | 0 | `pub queues: BTreeMap<(String, String), Queue>` (store.rs:22), `handles: HashMap<String, String>` (queue.rs:238), `attributes: BTreeMap<String, String>` (queue.rs:221) all untouched. |
| Ownership | 0 | `authorization_subject` still returns owned `(String, Option<String>)` (queue.rs:393-394); clones centralized, not removed. `queue_key` still allocates (queue_address.rs:14). |
| Comments | 0 | Banner removed (15e1892af); nine module docs added, each also repeated as `///` on the `mod` line (state.rs:231-232 vs queue_address.rs:3-4), following the baseline convention. Comment share 43.4% -> 43.4%. |
| Control flow | 0 | No function bodies changed; `ReceiveMessage::handle` still starts at ops/receive_message.rs:302 with the poll loop at :695. |
| Lint suppressions | 0 | clippy.txt identical (360 total, same breakdown); `#[allow(unused_imports)]` kept (ops/add_permission.rs:108). |
| Tests | 0 | No test changed; 280 pass in both. |
| Commit hygiene | +1 | Small, ordered, one-concern moves whose messages state why and say "moved as is", which checks out. 15e1892af also re-wraps docs of four earlier extractions (should have been in those commits). |
| Behavior risk | 0 | Moves verified line-for-line; `authorization_subject` body is byte-identical to the replaced expression (4e876f7e4, queue.rs:394). |

## Audit findings
| id | status | evidence |
|---|---|---|
| S-01 | untouched | |
| S-05 | untouched | |
| S-07 | untouched | |
| S-08 | untouched | |
| S-09 | untouched | |
| S-12/S-13 | partial | 4e876f7e4 folds the 18 `policy().map(str::to_string)` copies into queue.rs:393, but still allocates; owned `queue_key` and the 12 `expect`s remain |
| S-14/S-15 | untouched | |
| S-16/S-17 | untouched | |
| S-18 | untouched | |
| S-19 | untouched | |
| S-20 | untouched | |
| S-21 | untouched | |

## Best hunks
1. 4e876f7e4, `src/state/queue.rs:391-395` (with 18 call sites such as ops/delete_message.rs:212):
```rust
    /// The ARN and `Policy` this queue is authorized against, owned so the caller can
    /// release its borrow of the store before it evaluates them.
    pub fn authorization_subject(&self) -> (String, Option<String>) {
        (self.arn.clone(), self.policy().map(str::to_string))
    }
-        let (queue_arn, policy) = (queue.arn.clone(), queue.policy().map(str::to_string));
+        let (queue_arn, policy) = queue.authorization_subject();
```
Names a repeated concept once and documents why it is owned, with zero behavior risk.

2. 9b7a26dc7, `src/state/queue_address.rs:1-16`:
```rust
//! How a queue is addressed: its store key, its ARN and its url, and the reading of an ARN
//! or a url a request supplies.

use super::*;

pub fn queue_key(region: &str, name: &str) -> (String, String) {
    (region.to_string(), name.to_string())
}
```
Representative of the split: a store-independent concern pulled into its own small file, making the remaining state readable in parts.

## Worst hunks
1. 9b7a26dc7 (pattern repeated in all moves), `src/state.rs:231-234` plus `src/state/queue_address.rs:6`:
```rust
/// How a queue is addressed: its store key, its ARN and its url, and the reading of an
/// ARN or a url a request supplies.
mod queue_address;
pub use queue_address::*;
// and inside the module:
use super::*;
```
The doc sentence is written twice (outer `///` and inner `//!`, rendered twice by rustdoc), and glob imports both ways hide what each module depends on and exports.

## Silent edits check
Compared, per move commit, the sorted multiset of removed vs added non-blank lines: the only differences are the new module headers, `mod`/`pub use` lines, `use super::*`, the removed "Message movement tasks" banner, and visibility changes on `queue_arn_fields` (baseline state.rs:3542), `encoded_opaque_token` (:3789), `RECEIPT_HANDLE_FIELD_SEPARATOR`, `epoch_seconds`, and `MessageMoveTask::{is_of, in_progress, deliverable_by}` (:3290). Spot-read `queue_key`, `queue_arn_fields`, `deliverable_by` and `Queue::policy` against baseline: bodies identical. No silent edits found.
