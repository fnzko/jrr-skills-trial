# Review of run 3

## Summary
The attempt is almost entirely a decomposition of `src/state.rs` (4877 lines at baseline, 1569 at HEAD) into
thirteen new cohesive submodules (addressing, receipt_handle, fifo, limits, attribute_spec, attribute_validation,
parameter_refusals, queue_definition, message_content, message_move_task, plus methods moved into the existing
redrive and permissions modules), each re-exported from `state` with `pub use x::*` so no caller changes.
The only logic edit is a `Queue::authorization_subject()` helper replacing 18 identical
`(queue.arn.clone(), queue.policy().map(str::to_string))` expressions in the op handlers. A final commit runs
rustfmt over the crate, tests included (whitespace and trailing commas only). It leaves untouched every
type-level, ownership, error-handling and performance issue: string-keyed queues, the attribute string map,
`expect` after lookup, the long handlers, comment volume (43.4% -> 43.3%), and all 360 clippy findings.
Moves were verified token-for-token; the only non-move edits are visibility (`pub(super)`), new module headers
and one doc comment reattached to the module it describes.

## Commit classification
| hash | subject | kind | pure move? |
|---|---|---|---|
| 81fb4a81b | keep message attribute limits beside the rules that use them | move | yes |
| 64aa7d308 | move message digests and attribute sizing out of state.rs | move | yes (new module header, section header renamed) |
| 3b8df57b2 | give the message movement task record its own module | move | yes (+3 `pub(super)`) |
| 527777273 | split queue addressing and receipt handles out of state.rs | move | yes (+3 `pub(super)`) |
| f5c15d1c2 | split parameter refusals, attribute validation and queue rules out of state.rs | move | yes (mod list sorted, one doc comment em dashes -> commas) |
| dbb8cdea9 | move the attribute catalogue, store limits and redrive members out of state.rs | move | yes (+2 `pub(super)`) |
| 34e606d98 | place Queue and state methods beside the feature they implement | move | yes (new `impl` headers, +1 `pub(super)`) |
| 4c7588325 | name the (arn, policy) pair the authorization gate takes from a queue | dedupe | no |
| f54953a3b | run rustfmt over the crate | fmt | n/a (whitespace and 78 trailing commas only) |

## Scores
| dimension | score | evidence |
|---|---|---|
| Structure | +2 | state.rs 4877 -> 1569 lines; cohesive modules with one-line doc per `mod` (state.rs:160-200, e.g. `mod fifo` 175, `mod receipt_handle` 196); private items became `pub(super)` only, not `pub` (addressing.rs:172, receipt_handle.rs:169,311). Cost: 13 more `use super::*` and 9 more `pub use x::*` globs (glob metric 4.03 -> 4.97/KLOC) |
| Duplication | +1 | 4c7588325 state.rs:863-865 replaces 18 identical tuple expressions (e.g. ops/change_message_visibility.rs:258, ops/add_permission.rs:244); no other boilerplate (`lift`, `store_unavailable`) touched |
| Error handling | 0 | `expect("the queue was resolved out of this same store")` kept, e.g. ops/add_permission.rs:241-243 |
| Domain types | 0 | `(String, String)` keys and `(String, Option<String>)` return kept (state.rs:863, message_move_task.rs:159) |
| Ownership | 0 | helper still clones arn and policy (state.rs:864); clone/KLOC drop 8.42 -> 7.50 is only consolidation |
| Comments | 0 | banners removed (`// ----` in 3b8df57b2, 34e606d98; metric 0.61 -> 0.41), module docs added; volume 43.4% -> 43.3%, narration untouched |
| Control flow | 0 | no function bodies changed (token check of every move commit) |
| Lint suppressions | 0 | clippy.txt identical to baseline (360 total, same categories) |
| Tests | 0 | f54953a3b reformats tests only; 280 passed in both |
| Commit hygiene | +2 | one kind per commit, moves separate from the one logic change, bodies explain why (52 body lines); minor: fmt commit is last yet says it protects "every later edit" |
| Behavior risk | 0 | none found: moved bodies token-identical, helper is the same expression (4c7588325 state.rs:863-865), fmt whitespace-only |

## Audit findings
| id | status | evidence |
|---|---|---|
| S-01 | untouched | |
| S-05 | untouched | |
| S-07 | untouched | |
| S-08 | untouched | |
| S-09 | untouched | |
| S-12/S-13 | partial | 4c7588325 state.rs:863: the 18 `policy().map(str::to_string)` copies centralized, not removed; `queue_key` and the 12 `expect` remain |
| S-14/S-15 | untouched | |
| S-16/S-17 | untouched | |
| S-18 | untouched | state file split (34e606d98) but no handler or struct shrunk; receive_message handler intact |
| S-19 | untouched | |
| S-20 | untouched | |
| S-21 | untouched | |

## Best hunks
1. 4c7588325, src/state.rs:861-865 and every op file (e.g. ops/delete_message.rs:212):
```rust
    /// What the authorization gate needs of this queue, owned so the gate can run after
    /// the borrow of the store ends: the queue's ARN and its `Policy` document, if any.
    pub fn authorization_subject(&self) -> (String, Option<String>) {
        (self.arn.clone(), self.policy().map(str::to_string))
    }
-        let (queue_arn, policy) = (queue.arn.clone(), queue.policy().map(str::to_string));
+        let (queue_arn, policy) = queue.authorization_subject();
```
The only real dedupe: names the pair and records why it is owned, one place to change instead of 18.

2. 34e606d98, src/state/fifo.rs:1-16:
```rust
//! FIFO queue behaviour on the queue record: deduplication tokens, receive-attempt replay,
//! sequence numbers and the strict per-group delivery order.

use super::*;

impl Queue {
    pub fn deduplication_key(&self, group: &str, token: &str) -> (String, String) {
```
A 1000-line `impl Queue` split by feature into extra impl blocks with unchanged bodies; a reader finds FIFO logic in one 194-line file.

## Worst hunks
1. 527777273 / f5c15d1c2 / dbb8cdea9, src/state.rs:211-224:
```rust
pub use addressing::*;
pub use attribute_spec::*;
pub use attribute_validation::*;
pub use limits::*;
pub use message_attributes::*;
pub use message_content::*;
...
pub use receipt_handle::*;
pub use redrive::*;
```
Glob re-exports plus `use super::*` in each new module keep paths stable but hide where any name comes from, so the split does not show dependencies between modules.

## Silent edits check
Compared removed vs added lines (whitespace-normalized multiset) for all seven move commits. 81fb4a81b: zero
difference. Others: only differences are new module docs and SPDX headers, `mod`/`pub use` lines, `use super::*`,
`impl Queue {` / `impl ComAmazonawsSqsAmazonsqsState {` wrappers, removed `// ----` banners, and visibility raised
from private to `pub(super)` on `is_of`, `in_progress`, `deliverable_by`, `queue_arn_fields`, `encoded_opaque_token`,
`RECEIPT_HANDLE_FIELD_SEPARATOR`, `REDRIVE_PERMISSION_MEMBER`, `SOURCE_QUEUE_ARNS_MEMBER`, `forget_superseded_handles`.
f5c15d1c2 also rewrote the `authorization` module doc to replace em dashes with commas (wording otherwise identical).
Sampled `Queue::deduplication_key` (fifo.rs:16) and the message move task methods against baseline: identical. The fmt
commit's removed vs added text differs only by 78 commas and one brace pair (rustfmt trailing commas / arm block).
