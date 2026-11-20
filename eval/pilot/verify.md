# Verification of review pilot
## Score verdicts
| dimension | reviewer | verdict | corrected | evidence |
|---|---|---|---|---|
| Structure | +2 | confirmed | +2 | state.rs is 502 lines at HEAD; 33 files under src/state; pub(super)/pub(crate) 31 -> 41 and glob use 4.03 -> 6.13 match metrics.txt and git grep |
| Duplication | +2 | confirmed | +2 | d05096eeb ops/mod.rs:14 replaces 3 kms_refusal copies (diffstat 41+/101-); 6522083cb send_rules.rs adds one deduplication_id/message_group_fault/delay_fault/message_attributes_fault used by both send ops; 78d63acbd authorization.rs:366 used in 18 files |
| Error handling | +1 | confirmed | +1 | `expect("the queue was resolved` 12 -> 4 and `.expect(` 14 -> 6 in src (git grep); a9a73426d add_permission.rs:238 let-else; 0dee08f87 removed check was a same-args repeat of the one at old line ~346 |
| Domain types | 0 | zero confirmed | 0 | SendFault/BatchShapeFault are refusal enums replacing inline returns, not id or closed-set typing; `*_id/_arn: String` 2.91 -> 3.06/KLOC, enum defs 1.94 -> 2.06 only |
| Ownership | +1 | confirmed | +1 | 78d63acbd removes the arn clone and policy to_string pair per handler; clone() 8.42 -> 7.33 |
| Comments | 0 | zero confirmed | 0 | share 43.4 -> 43.5; new //! module docs in each move commit (e.g. 87407b705, 3801a277b) and macro docs offset the lost batch-KMS sentence in d05096eeb; ca7f390b0 only fixes links |
| Control flow | +2 | confirmed | +2 | c94f1c1ac receive_message.rs handle is now ~45 lines (304-348) with prepare_delivery/open_sealed_messages/take_batch; 8d9d40dcc, 7073cdda6, 668a4bb80 split the other big handlers |
| Lint suppressions | +1 | confirmed | +1 | cdba3b999 lib.rs removes 3 `#![allow]` lines (6 lints), `#[allow(` 3.78 -> 3.56/KLOC, clippy total 360 -> 354. No allow added elsewhere. Modest, +1 is right |
| Tests | 0 | zero confirmed | 0 | tests touched in 63 files but `git diff -w` is rustfmt wraps plus the 760408e03 rename and its doc line reword; no assertion or case changed in the sample read |
| Commit hygiene | +2 | confirmed | +2 | One concern per commit, reasoned bodies. Minor: ca7f390b0 repairs links the move commits broke, rustfmt is deferred to the last 2 commits touching every file |
| Behavior risk | 0 | confirmed | 0 | see below |
## Missed changes
- Every move commit (87407b705 .. 18514cd84) was scanned for added lines not present among removed lines: besides `use super::*`/`mod`/`pub(super)` they add new `///` and `//!` module docs and an SPDX header, nothing else. 12cfca4d0, 85a40e0de, 04ccc50e2, 798c51291 add zero new lines. Pure-move claim holds (docs added, not noted as such by review).
- 1177385fc adds a doc line with an em dash in state.rs (style, not scored). No missed score-relevant change found.
## Behavior risk
none found. Hunks checked: 0dee08f87 (first identical attempt-id check precedes, attempt_id not reassigned between), 6522083cb (token refusal order kept; standard-queue branch unreachable in batch, as review says; Missing/Invalid map to the same codes and texts as before), 78d63acbd (borrow replaces clone, store guard lifetime unchanged, denied touches iam/sts stores only), a9a73426d (queue_by_url_mut = queue_name_from_url + queue_mut, same QueueDoesNotExist), 7fc3d4cd0 (`map_err(attribute_error)` now returns the error type directly, same `.into()`), d05096eeb (match arms and text identical). Tests pass 280/280 as baseline.
## Audit status verdicts
- S-12/S-13 partial: confirmed (clones removed at 18 sites; queue_key still allocates).
- S-14/S-15 partial: confirmed (dead duplicate check removed in 0dee08f87; try_from sites consolidated).
- S-18 partial: confirmed (three handlers split; state structs untouched).
- All others untouched: spot-checked S-05 (Queue::integer), S-08 (POLL_INTERVAL at receive_message.rs:197,346), S-01; consistent.
## Hunks check
- Best 1 (receive_message.rs:318-331): exists as quoted.
- Best 2 (delete_message_batch.rs): exists at line ~194, but rustfmt split the call over lines, so the quote is not verbatim (`sqs::batch_shape_fault(` is on its own line). state/batch.rs:98 is correct.
- Worst 1 (ops/mod.rs:14-20): exists as quoted.
- Worst 2 (send_rules.rs:69-76): exists as quoted.
