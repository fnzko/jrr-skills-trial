# Review of run 5

## Summary
The attempt is almost entirely a decomposition of `src/state.rs` (4877 -> 697 lines) into 15 new or grown submodules under `src/state/` (addressing, attribute catalogue and validation, bounds, clock, digest, pagination, queue, queue_creation, receipt_handle, redrive, permissions, message_move_task, tags, wording, batch), done in 15 near-pure move commits whose only edits are module headers, `pub(super)` widening of a handful of private items, intra-doc link paths and removal of `// ----` banners. Three commits change code: a `macro_rules!` that replaces three 28-line copies of the KMS refusal mapping in the send/send-batch/receive ops, deletion of a duplicated, unreachable `ReceiveRequestAttemptId` check, and a rename of the `scratch_probe` test module. Everything the audit flagged about types, ownership, performance, long handlers and comment volume is left alone: `queue_key` still returns `(String, String)`, attributes are still a string map read through `integer()`, the 12 `expect("the queue was resolved out of this same store")` remain, and `receive_message::handle` is still one giant function. Tests and clippy output are identical to baseline.

## Commit classification
| hash | subject | kind | pure move? |
|---|---|---|---|
| 1a2f9c1e9 | move message digests and attribute sizing out of state.rs | move | yes (header + mod line) |
| 0dbc9ebaa | keep message attribute limits beside their validation | move | yes |
| 7f89c53a3 | move list paging helpers into their own module | move | yes |
| 25e6e2970 | move the store-bounds constants into state/bounds.rs | move | no (banner comment rewritten as `//!`, doc links requalified with `super::`) |
| ac48fd2ea | split attribute, queue-name and tag rules out of state.rs | move | yes |
| 3c9074c90 | give message moves, addressing, receipt handles and batches their own modules | move | no (5 items widened to `pub(super)`) |
| 915fe907a | move the queue attribute catalogue out of state.rs | move | no (2 consts widened to `pub(super)`) |
| d330a3d10 | move epoch renderings and span arithmetic into state/clock.rs | move | no (`epoch_seconds` widened) |
| 0cdf30d5a | collect refusal wording into state/wording.rs | move | no (section comment reworded as module doc) |
| 071d7303a | keep the store's message-move methods with the task they drive | move | yes |
| 1aa16f373 | move dead-letter redrive behaviour into the redrive module | move | yes |
| 6f1718eb1 | move queue permission methods into the permissions module | move | yes |
| ba53bff5d | move receipt-handle bookkeeping into the receipt_handle module | move | no (2 methods widened, new `impl Queue` block) |
| 73f18b643 | list state's submodules in one sorted, documented block | other | yes (reorder of mod/use lines) |
| b76235f6a | move the stored queue and message types into state/queue.rs | move | yes |
| cc73047a0 | name the batch-delete clock test for what it asserts | other (test rename) | no |
| 7f3a24488 | define the KMS refusal mapping once instead of three times | dedupe | no |
| 85203a443 | drop a second, unreachable ReceiveRequestAttemptId check | rewrite (dead code) | no |

## Scores
| dimension | score | evidence |
|---|---|---|
| Structure | +2 | state.rs 4877 -> 697 lines; cohesive modules declared and documented at `src/state.rs:160-213` (73f18b643, b76235f6a, 3c9074c90). Visibility creep minimal (9 items to `pub(super)`, e.g. `state/addressing.rs:171`). Caveat: every submodule does `use super::*` (`state/queue.rs:6`) and the parent re-exports all with `pub use x::*` (`state.rs:216-234`), so dependencies stay implicit. |
| Duplication | +1 | 7f3a24488: `kms_refusal_fn!` at `state/encryption.rs:263-299` replaces three copies; invoked at `ops/send_message.rs:606`, `ops/send_message_batch.rs:601`, `ops/receive_message.rs:691`. `store_unavailable`/`lift` copies untouched. |
| Error handling | +1 | 85203a443: duplicate unreachable `message_token_refusal` check removed after `ops/receive_message.rs:370`; the first check at `:341-350` stays. `expect` after lookup (`ops/untag_queue.rs:201,216`) untouched. |
| Domain types | 0 | `queue_key` still `(String, String)` at `state/addressing.rs:14`; `Queue::integer` at `state/queue.rs:409`. |
| Ownership | 0 | No clone/to_string changes (metrics per KLOC unchanged within rounding). |
| Comments | 0 | Comment share 43.4% before and after; only `// ----` banners dropped (0.61 -> 0.20 per KLOC) and turned into module docs (25e6e2970, 0cdf30d5a). |
| Control flow | 0 | No handler split; `receive_message::handle` still one function. |
| Lint suppressions | 0 | clippy.txt identical (360 lints, same breakdown); no `#[allow]` touched. |
| Tests | 0 | cc73047a0 renames `tests/all/scratch_probe.rs` -> `batch_delete_survives_visibility_lapse.rs` and edits 2 doc lines; body unchanged, nothing added or weakened. |
| Commit hygiene | +2 | 18 small commits, each one kind; move commits separated from the dedupe and dead-code commits; bodies state why and claim no behavior change (e.g. 7f3a24488, 85203a443). |
| Behavior risk | 0 | Removed check in 85203a443 is identical (same function, same member name, same input, no mutation between) to the one at `ops/receive_message.rs:341-350`; macro body matches the deleted match arm for arm. Test rename only changes the test path. |

## Audit findings
| id | status | evidence |
|---|---|---|
| S-01 | untouched | |
| S-05 | untouched | |
| S-07 | untouched | |
| S-08 | untouched | |
| S-09 | untouched | |
| S-12/S-13 | untouched | |
| S-14/S-15 | partial | 85203a443 removes the double `ReceiveRequestAttemptId` validation; `try_from(...).unwrap_or(1)` and `integer()` default left |
| S-16/S-17 | partial | 7f3a24488 dedupes per-op KMS refusal boilerplate; the named `store_unavailable`/`lift`/`#[allow(unused_imports)]` copies untouched |
| S-18 | untouched | state.rs file split, but giant handlers and the state struct unchanged |
| S-19 | untouched | |
| S-20 | untouched | |
| S-21 | untouched | (STS, outside crate) |

## Best hunks
7f3a24488, `src/state/encryption.rs:263-299` (excerpt):
```rust
macro_rules! kms_refusal_fn {
    ($error:ty, $modeled:ident) => {
        fn kms_refusal(refusal: $crate::state::encryption::KeyRefusal) -> $error {
            use $crate::state::encryption::KeyRefusal;
            match refusal {
                KeyRefusal::NotFound(message) => $modeled::KmsNotFound(KmsNotFound {
                    message: Some(message),
                }),
                ...
pub(crate) use kms_refusal_fn;
```
Three hand-synced 28-line copies become one definition next to `KeyRefusal`, with the "no trait spans the generated enums" rationale stated.

85203a443, `src/ops/receive_message.rs:370` (removed lines):
```rust
-        let attempt_id = input.receive_request_attempt_id;
-        if let Some(given) = &attempt_id {
-            if let Some(reason) = sqs::message_token_refusal("ReceiveRequestAttemptId", given) {
-                return Err(crate::scaffolded::Scaffolded::invalid_parameter_value(reason));
-            }
-        }
+        let attempt_id = input.receive_request_attempt_id;
```
Deletes a check that suggested two rules where one exists, exactly the audit's S-14 exemplar.

## Worst hunks
73f18b643 / b76235f6a, `src/state.rs:216-234` with `src/state/queue.rs:6`:
```rust
pub use addressing::*;
pub use attribute_catalog::*;
...
pub use wording::*;
// and in every submodule:
use super::*;
```
The split is physical only: each module glob-imports the whole parent (and through it every sibling), so a reader cannot see a module's dependencies and the glob-use rate rises from 4.03 to 5.29 per KLOC.

## Silent edits check
Extracted and diffed against `baseline:src/state.rs`: `Queue::hand_over` (58 lines), `Queue::materialise_due` (34), `Queue::granted_policy` (53), `dead_letter_due` (113), `advance_message_move_tasks`, `parse_queue_arn`, `md5_of_message_attributes`: all byte-identical. A sorted-line multiset comparison of each move commit shows the only non-moved lines are SPDX/module headers, `mod`/`pub use` lines, `use super::*`, nine `fn`/`const` signatures gaining `pub(super)`, intra-doc links requalified with `super::`, and banner comments rewritten as `//!` docs. No logic edits found in moved code.
