# Review of run 11
## Summary
The attempt split the 4,877-line `state.rs` into 14 submodules (state.rs is now 228 lines, the largest new
file `state/queue.rs` at 1,168), applied rustfmt to the crate, and then made five small semantic
refactors: a `MessageMoveStatus` enum, a `denied_for_queue` helper that removes the ARN/policy copies
in 17 handlers, a shared `batch_request_refusal` for the three batch operations (with unit tests),
resolve-once in the tag/permission handlers (8 of 12 `expect`s gone), and a dropped `to_string` in
`queue_by_url_mut`. It left alone the string-keyed store (`queue_key` -> `(String, String)`), the
attribute string map, the per-request sweeps and the 50 ms poll loop, the `lift`/`store_unavailable`
scaffold, lint suppressions, and comment volume. The split re-exports everything through glob
`pub use x::*` and each submodule starts with `use super::*`, so the namespace stays flat.

## Commit classification
| hash | subject | kind | pure move? |
|---|---|---|---|
| 394723c07 | split state.rs into focused submodules | move | yes (only `pub(super)` added, banners dropped) |
| 10a6c046d | name unused trait parameters instead of discarding them | other | no |
| 81f7f8619 | apply rustfmt to the crate | fmt | n/a (83 files, whitespace/reflow only) |
| 4dfb46eb2 | model message move task status as an enum | rewrite | no |
| 9da7cd6af | pass the queue to the authorization gate instead of its parts | dedupe | no |
| 04c9ce066 | share the whole-request checks of the three batch operations | dedupe | no |
| 2ea25385a | resolve the queue once in the tag and permission handlers | rewrite | no |
| 277cae972 | look a queue up by url without copying its name | other | no |

## Scores
| dimension | score | evidence |
|---|---|---|
| Structure | +2 | 394723c07: state.rs 4,877 -> 228 lines, 14 cohesive modules with one-line `//!` purpose each (state.rs:200-227); dented by glob `pub use *` re-exports and `use super::*` (state/store.rs:5), 40 `pub(super)` items |
| Duplication | +1 | 04c9ce066 refusal_messages.rs:325 replaces ~35 lines x3; 9da7cd6af authorization.rs:366 replaces a 9-line gate in 17 handlers; but each batch handler still carries a 28-line `map_err` match (send_message_batch.rs:260-289), `lift`/`store_unavailable` untouched |
| Error handling | +1 | 2ea25385a: `expect("the queue was resolved out of this same store")` 12 -> 4 (tag_queue.rs:189-193, add_permission.rs:231); 4 remain (set_queue_attributes.rs:249,335,344; start_message_move_task.rs:295) |
| Domain types | +1 | 4dfb46eb2 message_move_task.rs:68-89 `MessageMoveStatus` enum + Display replaces 4 `&str` consts; `(String, String)` keys untouched (queue_address.rs:13) |
| Ownership | +1 | 9da7cd6af drops `queue.arn.clone()` + `policy().map(str::to_string)` in 17 handlers; 277cae972 store.rs:131; clone() 8.42 -> 7.52 per KLOC |
| Comments | 0 | comment share 43.4% -> 43.4%; only banner lines removed (0.61 -> 0.10 per KLOC); new doc comments are contract-level |
| Control flow | +1 | 04c9ce066 and 2ea25385a shorten 7 handlers (e.g. untag_queue.rs:186-201 one lookup instead of three); giant `receive_message` handle untouched |
| Lint suppressions | 0 | clippy.txt identical to baseline; `#[allow(` 3.78 -> 3.76 per KLOC (dilution only) |
| Tests | +1 | 04c9ce066 refusal_messages.rs:356-400 adds 4 unit tests pinning batch messages and order; integration tests only reformatted in 81f7f8619 |
| Commit hygiene | +1 | each commit one kind, bodies say why and state invariants kept; minus: 83-file rustfmt commit mixes tests and src, and later 04c9ce066 adds lines over 100 cols (send_message_batch.rs:271) that fmt would rewrap |
| Behavior risk | 0 | checked: literal op names -> `Self::NAME` equal ("SendMessageBatch", "DeleteMessageBatch"); `queue.name` vs URL name in logs identical by exact-key lookup; Display text of enum matches old consts |

## Audit findings
| id | status | evidence |
|---|---|---|
| S-01 | untouched | |
| S-05 | untouched | |
| S-07 | untouched | |
| S-08 | untouched | |
| S-09 | untouched | |
| S-12/S-13 | partial | 2ea25385a untag_queue.rs:189 resolves once (exemplar fixed), 9da7cd6af removes the `policy().map(str::to_string)` copies; `queue_key` still allocates two Strings per lookup |
| S-14/S-15 | untouched | |
| S-16/S-17 | untouched | |
| S-18 | partial | 394723c07 splits the giant state file; 04c9ce066 shortens three batch handlers; `receive_message` 395-line handle untouched |
| S-19 | untouched | |
| S-20 | untouched | |
| S-21 | untouched | |

## Best hunks
2ea25385a, src/ops/tag_queue.rs:188-201
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
One lookup replaces a filter-and-copy plus two `expect`ed re-lookups, and the gate reads straight off the borrow.

04c9ce066, src/state/refusal_messages.rs:325-353
```rust
pub fn batch_request_refusal<'a>(
    operation: &str,
    ids: impl ExactSizeIterator<Item = &'a str>,
) -> Result<(), BatchRequestRefusal> {
    if ids.len() == 0 {
        return Err(BatchRequestRefusal::Empty(empty_batch_message(operation)));
    }
    ...
    let mut seen = BTreeSet::new();
    for id in ids {
        if !is_valid_batch_entry_id(id) { return Err(BatchRequestRefusal::InvalidEntryId(..)); }
        if !seen.insert(id) { return Err(BatchRequestRefusal::EntryIdsNotDistinct(format!("Id {id} repeated."))); }
    }
```
Three copies of the rule and its wording collapse into one typed decision, now covered by unit tests.

## Worst hunks
04c9ce066, src/ops/send_message_batch.rs:264-289 (same in delete/change-visibility batch)
```rust
        .map_err(|refusal| {
            use sqs::BatchRequestRefusal as Refusal;
            let modeled = match refusal {
                Refusal::Empty(message) => {
                    SendMessageBatchErrorModeled::EmptyBatchRequest(EmptyBatchRequest {
                        message: Some(message),
                    })
                }
                Refusal::TooManyEntries(message) => {
                    SendMessageBatchErrorModeled::TooManyEntriesInBatchRequest(TooManyEntriesInBatchRequest {
```
The four-arm mapping is still copied per handler and is not rustfmt-clean despite the earlier fmt commit.

394723c07, src/state.rs:215-227
```rust
pub use attribute_spec::*;
pub use attribute_validation::*;
pub use limits::*;
pub use message::*;
...
pub use store::*;
```
Every submodule is glob-reexported and glob-imports its parent, so the split leaves one flat namespace and hides real dependencies.

## Silent edits check
Compared with baseline `state.rs`, byte for byte: `integer` (attribute_spec.rs), `materialise_due` and
`delivery_order` (queue.rs), `receipt_handle_incarnation` (receipt_handle.rs), `check_attribute_value`
(attribute_validation.rs), `queue_name_from_url` (queue_address.rs): all identical at 394723c07. A
`--color-moved` scan of 394723c07 shows only `pub(super)` visibility on 9 items, removed banner comments,
module headers and the `mod`/`pub use` lines as non-moved. The rustfmt commit 81f7f8619 shows no token
change under a whitespace-insensitive word diff. No silent edits found.
