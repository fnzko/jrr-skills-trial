# Review of run 1

## Summary
The attempt is mostly a structural split: the 4877-line `src/state.rs` is cut into 16 new submodules (state.rs is now 505 lines) and 330 lines of query-compatible error plumbing move from `service.rs` into `service/query_compatible_errors.rs`. On top of the moves it makes four small real changes: the FIFO group/dedup/delay rules shared by `SendMessage` and `SendMessageBatch` (new `state/send_rules.rs` with a `SendFault` enum), shared delay and attribute-count refusal wording, the removal of a duplicate `ReceiveRequestAttemptId` check, and `batch_size` bound once in `ReceiveMessage`. It also drops 23 inert `#[allow(unused_imports)]` from op handlers, fixes a stale banner, updates the README and renames a test file. It leaves the data model alone: `(String, String)` queue keys, the string attribute map re-parsed on read, the per-request sweeps and sleep-loop polling, `expect` after lookup, the `store_unavailable`/`lift` boilerplate, and comment volume (still 43.4%). The split goes through `use super::*` in every submodule and `pub use sub::*` in the parent, so module dependencies are still implicit.

## Commit classification
| hash | subject | kind | pure move? |
|---|---|---|---|
| d26ae0b6b | test(sqs): rename scratch_probe to say what it covers | other | yes (file rename) |
| 51b9a4849 | move message attribute digests and sizes into message_attributes | move | yes |
| b53e414b0 | move CreateQueue recreate rules into state::recreate | move | yes |
| e25499c88 | move queue tag quotas and refusals into state::tags | move | yes |
| d0c4c54d0 | move queue attribute validation into state::attribute_validation | move | yes |
| 426a92b2a | move the queue attribute table into state::attribute_specs | move | no (2 consts made pub) |
| be9e61dc0 | move the message move task record into state::message_move_task | move | no (3 fns pub(super)) |
| 705efa041 | move refusal wording templates into state::refusal_messages | move | no (banner -> module doc) |
| f1dd838eb | move shared batch limits and wording into state::batch | move | yes |
| 3b2b728fe | move receipt handle handling into state::receipt_handle | move | no (2 items pub(super)) |
| 66a9f396f | move queue naming and addressing into state::queue_address | move | no (1 fn pub(super)) |
| 7034f0c8c | move list pagination into state::pagination | move | yes |
| 8b59accd7 | move the store ceilings into state::limits | move | no (banner -> doc, 2 refs reworded) |
| d3d156130 | move the queue store and its operations into state::store | move | yes |
| 0a915ed33 | move the Queue record and message types into state::queue and state::message | move | yes |
| 2b6689c53 | order the state module declarations and fix a stale banner | fmt | yes (one banner reworded) |
| c45ff569f | drop 23 unneeded allow(unused_imports) in op handlers | other | n/a |
| 87a40bbdb | share the delay and attribute-count refusals between single and batch send | dedupe | no |
| 062de3113 | validate ReceiveRequestAttemptId once in ReceiveMessage | rewrite | no |
| 06c379390 | name the ReceiveMessage batch size once | rewrite | no |
| f5f3170db | move the query-compatible error envelope out of service.rs | move | no (include_str path, pub(super)) |
| 6b937bf4c | docs(sqs): update the README layout | other | n/a |
| f534cd51a | judge a message's FIFO members and delay by one rule for single and batch sends | dedupe | no |

## Scores
| dimension | score | evidence |
|---|---|---|
| 1 Structure | +2 | d3d156130, 0a915ed33 and 12 others: state.rs 4877 -> 505 lines, cohesive modules (`state/store.rs`, `state/queue.rs`, `state/queue_address.rs`); f5f3170db `service/query_compatible_errors.rs`. Against it: glob `use super::*` in 18 submodules, `pub use x::*` block at `state.rs:202-221`, and 426a92b2a widens two consts to `pub` (`state/attribute_specs.rs:292,297`) |
| 2 Duplication | +1 | f534cd51a `state/send_rules.rs:44-111` replaces two copies of group/dedup/delay rules; 87a40bbdb `state/refusal_messages.rs:182-202`. Op boilerplate (`store_unavailable`, `lift`) untouched |
| 3 Error handling | +1 | 062de3113 drops the unreachable second `message_token_refusal` in `ops/receive_message.rs` (was baseline :374-379); f534cd51a typed `SendFault` (`state/send_rules.rs:15-20`) mapped per op (`ops/send_message.rs:537`). The 12 `expect` after lookup remain |
| 4 Domain types | 0 | `pub queues: BTreeMap<(String, String), Queue>` at `state/store.rs:22`, `handles: HashMap<String, String>` at `state/queue.rs:48` unchanged |
| 5 Ownership | 0 | `fifo_members` takes `Option<&str>` but re-allocates with `to_string` (`state/send_rules.rs:73,84`); no net change; `queue_key` still allocates (`state/queue_address.rs:14`) |
| 6 Comments | +1 | f534cd51a removes the duplicated FIFO rule comments from the batch path; 2b6689c53 fixes a stale banner (`state.rs:460`); 705efa041/8b59accd7 turn `// ----` banners into module docs. Comment share unchanged at 43.4%, line comments 84.8 -> 82.4 per KLOC |
| 7 Control flow | +1 | f534cd51a shrinks `SendMessage::handle` by ~60 lines (`ops/send_message.rs:373-383`); 06c379390 `batch_size` at `ops/receive_message.rs:316`. The 400-line receive handler is untouched |
| 8 Lint suppressions | +1 | c45ff569f removes 23 `#[allow(unused_imports)]`; clippy `allow_attributes` 142 -> 96, `allow_attributes_without_reason` 196 -> 150 |
| 9 Tests | 0 | d26ae0b6b renames `scratch_probe.rs` to `delete_message_batch_visibility_lapse.rs`, body unchanged; 280 tests pass in both |
| 10 Commit hygiene | +2 | 23 single-purpose commits, moves separated from edits, every visibility change named in the message (e.g. 426a92b2a, 3b2b728fe), messages give the why |
| 11 Behavior risk | 0 | f534cd51a adds a `!fifo` dedup check and a FIFO-positive-delay check to the batch path, but the handler refuses those cases whole first (`ops/send_message_batch.rs:381-400`); 062de3113: no early return between the kept check (baseline :345-351) and the removed one |

## Audit findings
| id | status | evidence |
|---|---|---|
| S-01 | untouched | |
| S-05 | untouched | |
| S-07 | untouched | |
| S-08 | untouched | |
| S-09 | untouched | |
| S-12/S-13 | untouched | |
| S-14/S-15 | partial | 062de3113 removes the duplicate ReceiveRequestAttemptId check; 06c379390 binds the `try_from(..).unwrap_or(1)` once (`ops/receive_message.rs:316`), default in `Queue::integer` kept |
| S-16/S-17 | partial | c45ff569f removes the inert allows in 23 op files; `store_unavailable`/`lift` copies remain |
| S-18 | partial | f534cd51a moves the FIFO rules out of `SendMessage::handle`; `ReceiveMessage::handle` still ~400 lines |
| S-19 | partial | f534cd51a deletes the second copy of the FIFO rule comments; overall comment share unchanged (43.4%) |
| S-20 | partial | 2b6689c53 fixes the stale "Message content: digests..." banner; 8b59accd7 fixes "above"/"Store bounds" references broken by the split |
| S-21 | untouched | STS only |

## Best hunks
f534cd51a, `src/state/send_rules.rs:98-111`
```rust
pub fn message_delay(queue: &Queue, requested: Option<i32>) -> Result<i64, SendFault> {
    if let Some(requested) = requested.filter(|delay| queue.is_fifo() && *delay > 0) {
        return Err(SendFault::InvalidParameterValue(invalid_parameter_message(
            &requested.to_string(),
            DELAY_SECONDS,
            QUEUE_TYPE_PARAMETER_REASON,
        )));
    }
    let delay = requested.map_or_else(|| queue.integer(DELAY_SECONDS), i64::from);
    match delay_seconds_refusal(delay) {
        Some(reason) => Err(SendFault::InvalidParameterValue(reason)),
        None => Ok(delay),
    }
}
```
One rule for single and batch sends, returning a neutral fault that each op maps to its own error shape, so the two paths can no longer drift.

06c379390, `src/ops/receive_message.rs:316` and `:621`
```rust
        let batch_size = usize::try_from(max_messages).unwrap_or(1);
...
                    queue.hand_over(now, batch_size, invisible_until)
```
Four multi-line repetitions of a conversion after the range check become one named binding.

## Worst hunks
d3d156130 and siblings, `src/state.rs:202-221` (plus `use super::*;` at the top of each submodule)
```rust
pub use attribute_specs::*;
pub use attribute_validation::*;
pub use batch::*;
...
pub use store::*;
pub use tags::*;
```
Glob re-exports in both directions keep every item reachable from everywhere, so the split shows no real module boundaries or dependencies, and it widens visibility (two consts made `pub` at `state/attribute_specs.rs:292,297`).

## Silent edits check
I compared each move commit as a multiset of trimmed lines (added against removed). Commits 51b9a4849, d3d156130, 0a915ed33, f1dd838eb, 7034f0c8c, e25499c88, d0c4c54d0 and b53e414b0 add only headers, module docs, `mod`/`use` lines. The remaining moves differ only by the visibility changes and comment rewordings their messages declare (pub/pub(super) on 2 consts, 3 MessageMoveTask methods, `encoded_opaque_token`, `RECEIPT_HANDLE_FIELD_SEPARATOR`, `queue_arn_fields`, `QueryCompatibleErrors`, `refusal_for`, and the `include_str!` path). I also diffed 40-line bodies against baseline `state.rs` for `queue_key` (baseline :3385), `Queue::integer` (:1440), `Queue::delivery_order` (:1580) and `advance_message_move_tasks` (:2840). All four are identical. I found no undeclared edits.
