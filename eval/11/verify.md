# Verification of review 11
## Score verdicts
| dimension | reviewer | verdict | corrected | evidence |
|---|---|---|---|---|
| Structure | +2 | confirmed | +2 | 394723c07 splits state.rs into 14 modules; glob `pub use *` at state.rs:215-227 and `use super::*` are real and metrics show glob use 4.03 -> 5.49 per KLOC, a visible dent but not enough to drop below +2 |
| Duplication | +1 | confirmed | +1 | 04c9ce066 removes ~35 lines x3 into batch_request_refusal; 9da7cd6af adds denied_for_queue (+11 lines, -17 handler copies); 4-arm map_err remains per handler as claimed |
| Error handling | +1 | confirmed | +1 | 2ea25385a drops 8 expects in add_permission/remove_permission/tag_queue/untag_queue; metrics .expect 0.71 -> 0.30 per KLOC matches |
| Domain types | +1 | confirmed | +1 | 4dfb46eb2 message_move_task.rs MessageMoveStatus enum replaces 4 &str consts, status() returns the enum, ==comparisons in store.rs and cancel_message_move_task.rs; wire text still stringified at store.rs (`status.to_string()`). Small but real; enum defs 1.94 -> 2.03 |
| Ownership | +1 | confirmed | +1 | 9da7cd6af drops `queue.arn.clone()` and `policy().map(str::to_string)`; 277cae972 store.rs:131 drops `.to_string()`; clone() 8.42 -> 7.52 |
| Comments | 0 | zero confirmed | 0 | comment share 43.4 -> 43.4; only banner lines 0.61 -> 0.10; new docs are contract-level |
| Control flow | +1 | confirmed | +1 | 2ea25385a shortens four handlers to one lookup; 04c9ce066 shortens three. Modest, not broad |
| Lint suppressions | 0 | zero confirmed | 0 | clippy.txt diff against baseline is empty; `#[allow(` change is density dilution only |
| Tests | +1 | confirmed | +1 | 04c9ce066 refusal_messages.rs adds 4 real unit tests (empty, 11 entries, order, valid); test density 0.26 -> 0.46. Integration tests only reflowed by 81f7f8619 (inspected word-diff: sorted `mod` order and wrapping only) |
| Commit hygiene | +1 | confirmed | +1 | one kind per commit with why-bodies; 83-file fmt commit mixes src and tests as noted |
| Behavior risk | 0 | confirmed | 0 | see below |
## Missed changes
none of substance. 10a6c046d (service.rs) renames `req`/`error` to `_req`/`_error` and removes two `let _ =` (metric `let _ =` 0.20 -> 0.10); the review lists it as "other" without scoring it. Negligible, not worth a score change. 81f7f8619 also reorders `mod` lines in tests (query_compatible_error_envelope moved); harmless.
## Behavior risk
none found. Checked: 04c9ce066 keeps check order (empty, >10, then per-entry invalid before repeated) and message text byte for byte; `Self::NAME` vs literal names equal; 2ea25385a logs `queue.name` instead of URL-derived name (equal under exact-key lookup) and `queue_by_url_mut` equals the old filter(holds_queue) + queue_mut; 4dfb46eb2 Display strings equal old consts; 277cae972 only drops a copy. Side-effect order in the handlers (authorization before mutation) is unchanged.
## Audit status verdicts
S-12/S-13 partial: confirmed (untag_queue.rs single lookup; policy copy removed; queue_key still allocates).
S-18 partial: confirmed (state split, three batch handlers shortened, receive_message untouched). Mildly generous since S-18 is about handler size, but the batch handlers did shrink.
All others untouched: no evidence of a change found.
## Hunks check
Best 1 (tag_queue.rs:188-201): exists as quoted. Best 2 (refusal_messages.rs:325-353): exists; the quote elides the TooManyEntries branch and compresses the if-bodies onto single lines, fine as abbreviation.
Worst 1 (send_message_batch.rs:264-289): exists; the unwrapped `TooManyEntriesInBatchRequest {` line at 271 is over 100 columns, as claimed. Worst 2 (state.rs:215-227): exists as quoted.
