# Verification of review 5
## Score verdicts
| dimension | reviewer | verdict | corrected | evidence |
|---|---|---|---|---|
| Structure | +2 | confirmed | +2 | state.rs 4877 -> 697 (wc); mod block `src/state.rs:160-214`, `pub use` at 216-234; glob `use super::*` (`state/queue.rs:6`) tempers it but the split is broad. |
| Duplication | +1 | confirmed | +1 | 7f3a24488: three 28-line `kms_refusal` bodies replaced by `kms_refusal_fn!` (`state/encryption.rs:263-299`; invocations `send_message.rs:606`, `send_message_batch.rs:601`, `receive_message.rs:691`). Match arms are identical modulo the enum name. |
| Error handling | +1 | confirmed | +1 | 85203a443: the removed check is the same call (`message_token_refusal("ReceiveRequestAttemptId", ..)`) already made at `receive_message.rs:341-350`; no mutation between them. Rubric names unreachable defensive branches. Small but real. |
| Domain types | 0 | zero confirmed | 0 | `queue_key` still returns `(String, String)` (`state/addressing.rs:14`); `integer()` at `state/queue.rs:409`. |
| Ownership | 0 | zero confirmed | 0 | metrics.txt clone()/to_string() only shift by rounding (LOC denominator); no hunk touches them. |
| Comments | 0 | zero confirmed | 0 | Line comments 84.84 -> 83.71 /KLOC, banners 0.61 -> 0.20, em dash 48.93 -> 48.34; only rewrites of module headers and banner conversion. Negligible. |
| Control flow | 0 | zero confirmed | 0 | No handler touched except the dead-check removal; no let-else or split. |
| Lint suppressions | 0 | zero confirmed | 0 | clippy.txt identical to baseline (empty diff); `#[allow(` rate moves only through LOC. |
| Tests | 0 | zero confirmed | 0 | cc73047a0 is a rename plus a 2-line doc reword (96% similarity); test.txt 280 pass both sides. |
| Commit hygiene | +2 | confirmed | +2 | 18 commits, each one kind, bodies give the why (7f3a24488, 85203a443, cc73047a0). |
| Behavior risk | 0 | confirmed | 0 | See below. |

## Missed changes
- Rubric-relevant: none. A normalized-line multiset check of commits 1-15 shows non-moved lines only as headers, `mod`/`pub use`/`use super::*`, `pub(super)` widenings, doc-link requalification, banner removals.
- Minor accuracy slips in the review (do not change any score): the number of items widened to `pub(super)` is 11 (queue_arn_fields, is_of, in_progress, deliverable_by, encoded_opaque_token, RECEIPT_HANDLE_FIELD_SEPARATOR, REDRIVE_PERMISSION_MEMBER, SOURCE_QUEUE_ARNS_MEMBER, epoch_seconds, forget_superseded_handles, remember_deleted_handle), not 9.
- 25e6e2970 also deletes the comment "The per-queue tag bounds live with the other tag quotas above..." (a cross-reference), and 85203a443 deletes a rationale comment together with the duplicate check. Neither is flagged as a comment change; both negligible.

## Behavior risk
none found. Macro body matches each deleted match arm; its inner `use $crate::ops::types::{Kms*}` replaces the module-level imports, same types. Doc comment moved inside the generated fn. The removed attempt-id check was dominated by the earlier identical check (same input field, same error constructor `Scaffolded::invalid_parameter_value`). `pub(super)` widenings only add visibility inside `state`.

## Audit status verdicts
- S-14/S-15 partial: confirmed. The audit text names "ReceiveRequestAttemptId validated twice in one handler"; 85203a443 removes it, the `try_from(..).unwrap_or(1)` sites remain.
- S-16/S-17 partial: overstated. The finding is about `store_unavailable`, `lift`, `state_unavailable` and `#[allow(unused_imports)] use super::types::*;` (see audit lines 426-428). The `kms_refusal` copies are not in that finding. Should be untouched (a related dedupe, not the named pattern).
- S-18 untouched: confirmed (receive_message handler unchanged; finding is about handler length).
- All others untouched: no contrary evidence.

## Hunks check
- Best 1: `state/encryption.rs:263-299` exists as quoted (the excerpt drops the inner doc comment and `use` lines, marked by `...`). Valid.
- Best 2: the removal at `receive_message.rs` (old lines ~370-378) matches the diff. The claim "exactly the audit's S-14 exemplar" is fair for the SQS double-validation mention, though the audit's exemplars are DynamoDB.
- Worst: `state.rs:216-234` `pub use ...::*` and `state/queue.rs:6` `use super::*` exist; glob use rate 4.03 -> 5.29 per KLOC matches metrics.txt. Valid.
