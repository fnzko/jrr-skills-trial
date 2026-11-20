# Verification of review 1
## Score verdicts
| dimension | reviewer | verdict | corrected | evidence |
|---|---|---|---|---|
| Structure | +2 | confirmed (generous) | +2 | state.rs 4877 -> 505 lines (`state.rs:202-221` re-export block verified); glob use 4.03 -> 5.53 per KLOC in metrics.txt and two consts made pub (`state/attribute_specs.rs:292,297`) argue for +1, but the split is broad and cohesive |
| Duplication | +1 | confirmed | +1 | f534cd51a `state/send_rules.rs:44-111` replaces two copies; 87a40bbdb `state/refusal_messages.rs:178-202` byte-identical wording |
| Error handling | +1 | confirmed | +1 | 062de3113 removes the real duplicate attempt-id check (first one kept at `ops/receive_message.rs:341-350`); `SendFault::InvalidParameterValue(String)` is still stringly, so only the dead branch carries the score |
| Domain types | 0 | zero confirmed | 0 | queue key, `handles` map unchanged (`state/store.rs:22`, `state/queue.rs:48`); `SendFault` is an error enum, not a domain type |
| Ownership | 0 | zero confirmed | 0 | clone() 8.42 -> 8.17, to_string 6.68 -> 6.65: negligible; `fifo_members` still allocates the same strings |
| Comments | +1 | too high | 0 | the "stale banner" fixed in 2b6689c53 (`state.rs:460`) and the "above"/"Store bounds" references in 8b59accd7 were made stale by this attempt's own moves; at baseline `state.rs:4646` the banner was true. Share still 43.4%; line comments 84.8 -> 82.4 but inner doc 126.5 -> 128.5; the FIFO rule prose is kept in `send_rules.rs` docs, banners became module docs (neutral) |
| Control flow | +1 | confirmed (borderline) | +1 | f534cd51a shrinks `SendMessage::handle` (`ops/send_message.rs:373-383`); 06c379390 binds `batch_size` once (`ops/receive_message.rs:316,621`); 400-line receive handler untouched |
| Lint suppressions | +1 | confirmed | +1 | c45ff569f: `#[allow(unused_imports)]` 47 -> 24 in src; metrics `#[allow(` 3.78 -> 2.59; clippy.txt 142 -> 96 and 196 -> 150 |
| Tests | 0 | zero confirmed | 0 | d26ae0b6b only renames the module and file plus one `//!` word; no test diff elsewhere |
| Commit hygiene | +2 | confirmed (borderline) | +2 | 23 single-purpose commits; 2b6689c53 mixes a reorder with a banner reword, and its "stale" rationale is wrong (see Comments) |
| Behavior risk | 0 | confirmed | 0 | see below |
## Missed changes
none. I diffed every move commit as a multiset of trimmed added vs removed lines: only mod/use lines, new module docs, banners turned into docs, and the declared visibility widenings (`pub` on 2 consts, `pub(super)` on 3 MessageMoveTask fns, `encoded_opaque_token`, `RECEIPT_HANDLE_FIELD_SEPARATOR`, `queue_arn_fields`, `QueryCompatibleErrors`, `refusal_for`, `include_str!` path). 51b9a4849 has no content diff. Clippy and metrics deltas match the reviewed changes.
## Behavior risk
none found. f534cd51a keeps the check order (group, token, delay, then attribute count). The two extra checks in the shared rule (token on standard queue, positive delay on FIFO) are unreachable in batch because the handler refuses those whole first (`ops/send_message_batch.rs:381-400`, positive delay on FIFO, token on standard). 062de3113 drops the second check; the first (`ops/receive_message.rs:341-350`) returns first. 06c379390 reuses the already range-checked value.
## Audit status verdicts
- S-14/S-15 partial: confirmed (attempt-id duplicate removed, 4 `try_from` bound once; one `unwrap_or(1)` kept).
- S-16/S-17 partial: confirmed (23 inert allows removed; `store_unavailable`/`lift` remain).
- S-18 partial: overstated. The named exemplar `receive_message.rs` handler is untouched; only ~60 lines left a different handler, via dedupe.
- S-19 partial: overstated, effectively untouched. Comment share unchanged at 43.4%, the exemplar narration style is unchanged, and no comment volume was cut.
- S-20 partial: overstated. The banner fixed was stale only because of this attempt's move; the audit's contradicting comments are untouched (`this slice` count is 0 in SQS at baseline and HEAD).
- Other ids untouched: no counter-evidence found.
## Hunks check
Best 1 `state/send_rules.rs:98-111` exists as quoted. Best 2 `ops/receive_message.rs:316` and `:621` exist as quoted. Worst `state.rs:202-221` exists as quoted (list of `pub use x::*`, with the `use super::*` count of 18 submodules confirmed). Line citations `state/attribute_specs.rs:292,297`, `state/store.rs:22`, `state/queue.rs:48`, `state/queue_address.rs:14` all match.
