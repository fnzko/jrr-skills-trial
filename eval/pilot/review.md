# Review of run pilot
## Summary
The attempt splits the 4,877-line `src/state.rs` into 30 concern-named modules under `src/state/` (state.rs is now 502 lines), in 31 move commits that are pure apart from `use super::*;` and private items widened to `pub(super)`. It then makes 13 commits that change code. They break up the three biggest handlers (ReceiveMessage, SendMessage, SendMessageBatch) and StartMessageMoveTask's ARN steps. They define once things that were copied: the KMS refusal mapping (3 copies), the batch-shape refusals (3), the send member rules (2), the attribute refusal wording (2) and the queue authorization call (18). They also remove 8 of the 12 `expect`s that came after a lookup and drop a ReceiveRequestAttemptId check that could never fire. Crate-wide lint allows are dropped, one test file is renamed, and rustfmt runs as the last two commits. It leaves the audit's type and performance findings alone: `(String, String)` queue keys, the string attribute map re-parsed by `integer()`, the store-wide sweep in `publish_queue_metrics`, the 50 ms poll loop, the linear scans in `delivery_order`, the comment volume (43.4% -> 43.5%), and the generated `#[allow(unused_imports)]` boilerplate.

## Commit classification
| hash | subject | kind | pure move? |
|---|---|---|---|
| 87407b705..f5a20c63a (16 commits) | move limits, pagination, fifo, tags, queue_address, batch, receipt_handle, message_move_task, clock, queue_refusals, parameter_refusals, attribute_specs, attributes, recreate, message_content, redrive | move | yes (+`use super::*`, a few items made `pub(super)`) |
| 12cfca4d0, 85a40e0de, d70856c84, ec8b5ac23 | move message_attributes, body checks, queue_lifecycle, message | move | yes |
| 1177385fc | order module list alphabetically | other | yes |
| f8b859185..169b5c815 (5) | split `impl Queue` into queue_policy, queue_receipt_handles, queue_deduplication, queue_delivery, queue_contents | move | yes (+`pub(super)`) |
| 56f4a5295, 42da1c76d, 04ccc50e2, 18514cd84 | move dead_letter, move-task methods, count_as_i64, Queue record | move | yes |
| 798c51291 | keep module list sorted | other | yes |
| ca7f390b0 | fix intra-doc links | other (docs) | no |
| 0dee08f87 | drop unreachable ReceiveRequestAttemptId check | rewrite | no |
| c94f1c1ac | break ReceiveMessage::handle into phases | rewrite | no |
| d05096eeb | define KMS refusal mapping once | dedupe | no |
| 6522083cb | one set of send member rules | dedupe | no |
| 8d9d40dcc | pull SendMessage judging out of handle | rewrite | no |
| c50fbb22b | batch shape judged in one place | dedupe | no |
| 78d63acbd | authorize queue-scoped call with one helper | dedupe | no |
| 7073cdda6 | split SendMessageBatch::handle | rewrite | no |
| 668a4bb80 | name ARN/destination steps of StartMessageMoveTask | rewrite | no |
| 7fc3d4cd0 | attribute refusal wording once | dedupe | no |
| 760408e03 | rename scratch_probe test suite | other (test) | yes (rename + doc line) |
| 2daa8468d | README layout | other (docs) | no |
| cdba3b999 | drop crate-wide lint allows | other | no |
| a9a73426d | look queue up once in tag/permission handlers | rewrite | no |
| c485f63ab | rustfmt crate | fmt | n/a |
| e64f4e138 | rustfmt ops | fmt | n/a |

## Scores
| dimension | score | evidence |
|---|---|---|
| Structure | +2 | 30 cohesive modules, `src/state.rs:170-267`. state.rs goes from 4877 to 502 lines and the largest file is `state/authorization.rs` (983). Against it: `use super::*` in every module (glob use 4.03 -> 6.13/KLOC), `impl Queue` spread over 6 sibling files, and `pub(super)`/`pub(crate)` up from 31 to 41 |
| Duplication | +2 | d05096eeb `ops/mod.rs:14`; c50fbb22b `state/batch.rs:98`; 6522083cb `state/send_rules.rs:49-119`; 78d63acbd `state/authorization.rs:366` (18 call sites); 7fc3d4cd0 `ops/mod.rs:84` |
| Error handling | +1 | a9a73426d `ops/add_permission.rs:238` (let-else, two `expect`s gone): resolved-queue `expect`s go from 12 to 4. 0dee08f87 removes the dead re-check. `.expect(` 0.71 -> 0.30/KLOC |
| Domain types | 0 | `SendFault`/`BatchShapeFault` (`state/send_rules.rs:16`, `state/batch.rs:82`) are refusal enums and `ReceiveRequest` (`ops/receive_message.rs:354`) is a parsed request, but ids, `(String, String)` keys and the attribute string map are unchanged |
| Ownership | +1 | 78d63acbd: 18 `queue.arn.clone()` + `policy().map(str::to_string)` copies removed (`ops/add_permission.rs:241-249`). clone() 8.42 -> 7.33/KLOC |
| Comments | 0 | Comment share 43.4% -> 43.5%. Comments moved with their code, macro and module docs added (`ops/mod.rs:5-13`). The batch-only KMS rationale ("the key belongs to the QUEUE") was lost in d05096eeb |
| Control flow | +2 | c94f1c1ac: the 400-line handle becomes a 45-line loop (`ops/receive_message.rs:304-348`) plus `prepare_delivery`/`open_sealed_messages`/`take_batch`/`replay_attempt`. 8d9d40dcc `ops/send_message.rs:478` `judge`. 7073cdda6 `ops/send_message_batch.rs:456,494`. 668a4bb80 `ops/start_message_move_task.rs:426,448` |
| Lint suppressions | +1 | cdba3b999 `src/lib.rs:1-5`: 6 crate-wide allows removed. clippy `allow_attributes_without_reason` 196 -> 190, other lints unchanged |
| Tests | 0 | Only the rename in 760408e03 (`tests/all.rs:16`) and rustfmt (c485f63ab). No test body changed |
| Commit hygiene | +2 | One concern per commit, messages give the reason and the preserved order of checks (e.g. 6522083cb, c50fbb22b). Small minus: a9a73426d left long unformatted lines that e64f4e138 fixes later |
| Behavior risk | 0 | Checked 0dee08f87 (first check identical, `ops/receive_message.rs:401-408`), c94f1c1ac (lock scopes and order kept), a9a73426d (`queue_by_url_mut` = `queue_name_from_url`+`holds_queue`, `state.rs:392`), c50fbb22b (`Self::NAME` == "DeleteMessageBatch"), 668a4bb80 (parameter names kept). `send_rules.rs:71` adds a standard-queue branch to the batch path that the handler guard (`ops/send_message_batch.rs:348-357`) makes unreachable |

## Audit findings
| id | status | evidence |
|---|---|---|
| S-01 | untouched | `queue_key` still returns `(String, String)`, `state/queue_address.rs:29` |
| S-05 | untouched | `Queue::integer` still re-parses the string map, `state/queue.rs:282-291` |
| S-07 | untouched | `service.rs:508` still calls `publish_queue_metrics` on every request |
| S-08 | untouched | `POLL_INTERVAL` sleep loop, `ops/receive_message.rs:197,346` |
| S-09 | untouched | `runs.iter_mut().find`, `state/queue_delivery.rs:53` |
| S-12/S-13 | partial | 78d63acbd removes all 18 `policy().map(str::to_string)` copies; a9a73426d resolves the queue once in untag/tag/permission handlers. `queue_key` still allocates per lookup |
| S-14/S-15 | partial | 0dee08f87 drops the duplicate ReceiveRequestAttemptId check. `usize::try_from(max).unwrap_or(1)` goes from 4 sites to 1 (`ops/receive_message.rs:434`) |
| S-16/S-17 | untouched | 47 `#[allow(unused_imports)]` in ops, same as baseline |
| S-18 | partial | ReceiveMessage/SendMessage/SendMessageBatch handlers split (c94f1c1ac, 8d9d40dcc, 7073cdda6); state structs untouched |
| S-19 | untouched | `state/queue.rs:132` still narrates `sequenced: 0`; comment share unchanged |
| S-20 | untouched | no scope-narration comments were removed or corrected |
| S-21 | untouched | STS, outside this crate |

## Best hunks
c94f1c1ac, `ops/receive_message.rs:318-331`:
```rust
loop {
    let now = SystemTime::now();
    let delivering = prepare_delivery(&store, &region, &request, now);
    if delivering {
        self.open_sealed_messages(&store, ctx, &region, &request, now)?;
    }
    let (messages, wait) =
        match self.take_batch(&store, ctx, &region, &request, delivering, now)? {
            Taken::Replayed(output) => return Ok(output),
            Taken::Polled { messages, wait } => (messages, wait),
        };
```
The 400-line loop now reads as named phases with the same lock scopes, and the four `usize::try_from(max_messages)` conversions collapse into one field of `ReceiveRequest`.

c50fbb22b, `ops/delete_message_batch.rs` (same in the other two batch ops):
```rust
if let Some(fault) =
    sqs::batch_shape_fault(Self::NAME, input.entries.iter().map(|entry| entry.id.as_str()))
{
    return Err(batch_shape_refusal(fault));
}
```
This replaces three 45-line copies of the whole-request checks with one function (`state/batch.rs:98`) that keeps the order of checks.

## Worst hunks
d05096eeb / c50fbb22b / 7fc3d4cd0, `ops/mod.rs:14-20`:
```rust
macro_rules! define_kms_refusal {
    ($modeled:ident, $error:ident) => {
        fn kms_refusal(refusal: $crate::state::encryption::KeyRefusal) -> $error {
            use $crate::state::encryption::KeyRefusal;
            match refusal {
                KeyRefusal::NotFound(message) => $modeled::KmsNotFound(KmsNotFound {
```
Three macros define free functions that name `KmsNotFound`, `EmptyBatchRequest` and the like without a path, so they only work through each op file's `use super::types::*` glob. A reader has to expand the macro to find `kms_refusal`.

6522083cb, `state/send_rules.rs:69-76`:
```rust
Some(given) => {
    if !fifo {
        return Err(SendFault::Invalid(invalid_parameter_message(
            given,
            "MessageDeduplicationId",
            QUEUE_TYPE_PARAMETER_REASON,
        )));
    }
```
Sharing the rule adds a branch the batch path can never reach, because the handler refuses the whole batch first. That is the kind of dead defensive code S-14 describes, now explained by a comment at the batch call site.

## Silent edits check
I compared these moved bodies against `baseline:src/state.rs` with whitespace ignored: `Queue::integer`, `queue_key`, `dead_letter_due`, `hand_over`, `receipt_handle_incarnation` and `validate_attributes` are identical, and `delivery_order` differs only by a rustfmt line wrap. Scanning the 31 move commits for lines outside the moves (`--color-moved` with indentation allowed) shows only `mod`/`pub use` lines, `use super::*;`, `impl X {` headers and private items widened to `pub(super)`. The baseline ReceiveMessage loop matches the split phases statement for statement: the replay branch moved intact into `replay_attempt`, and `authorization::denied` became `denied_for_queue` with the same arguments. No silent edits found.
