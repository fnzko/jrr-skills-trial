# Review of run 8
## Summary
The attempt breaks the 4,877-line `state.rs` into 21 cohesive submodules (addressing, receipt handles, limits, queue record split by concern under `state/queue/`, dead-letter and move-task store impls, refusals) through 18 near-verbatim move commits. `state.rs` ends at 477 lines, and `pub use x::*` re-exports keep the flat `sqs::` paths. It then drops six crate-level clippy allows and relocates stranded banners and rationale comments. Four rewrite commits split `ReceiveMessage::handle` into named steps, pull SendMessage's parameter rules into `prepare`, name the request-level stages of SendMessageBatch, and dedupe the StartMessageMoveTask ARN match. Domain types (`(String, String)` keys, the string attribute map), ownership, the long-poll loop, linear scans and the op-file scaffold are untouched, and so are tests.

## Commit classification
| hash | subject | kind | pure move? |
|---|---|---|---|
| c5f1c5be8 | move queue addressing out of state.rs | move | yes (imports, one pub(super), stray doc fixed in 440ac3623) |
| cc927933c | move receipt handle rules | move | yes (2 pub(super)) |
| adcd08d90 | move message move task model | move | yes (3 pub(super)) |
| d8c0b365b | move listing pagination | move | yes |
| 4b0cb88d3 | move clock helpers | move | yes |
| 65eb929ad | move message digests | move | yes |
| 9b888ebfe | gather message attribute rules | move | yes |
| 54a427532 | move queue tag rules | move | yes |
| f45ada8d0 | move queue attribute definitions | move | yes (banners dropped) |
| ef29f9ba2 | move attribute and queue name validation | move | yes |
| 4c1261d8f | gather store bounds in limits | move | yes (module doc reworded) |
| 013efcd3b | move the queue record | move | yes |
| eca16422f | split the Queue impl by concern | move | yes (5 impl blocks, 1 pub(super)) |
| 4803000c1 | split the store impl by concern | move | yes |
| 77c3d9892 | move batch entry rules | move | yes |
| 369b44d4c | move message content rules | move | yes |
| 65f97b3b1 | retention windows to limits | move | yes |
| 440ac3623 | move refusal wording, order mod decls | move | yes (plus reordering) |
| 586edbacb | drop blanket lint allows | other | no |
| 00bab2914 | tidy leftovers of the split | other (comments) | no |
| a41704580 | break ReceiveMessage::handle into steps | rewrite | no |
| 5048c7b0a | extract SendMessage rules into prepare | rewrite | no (verbatim body extraction) |
| 63a315d74 | name request-level steps of SendMessageBatch | rewrite | no (verbatim extraction) |
| 5108af4fd | share ARN classification of StartMessageMoveTask | dedupe | no |

## Scores
| dimension | score | evidence |
|---|---|---|
| Structure | +2 | 013efcd3b..440ac3623: state.rs 4877 -> 477 lines (`state.rs:191-225` mod list); `state/queue/{fifo,handles,delivery,bounds,access_policy}.rs` by concern. Only 11 `pub(super)` widenings (e.g. `state/message_move.rs:162`), but 18 glob `pub use` lines keep one flat namespace |
| Duplication | +1 | 5108af4fd `ops/start_message_move_task.rs:450` `queue_named_by` replaces two copies of the 3-way match. a41704580 `ops/receive_message.rs:446` computes `usize::try_from(max_messages)` once, not 4 times |
| Error handling | +1 | a41704580 drops the second, unreachable `ReceiveRequestAttemptId` check (the same predicate already returned at `ops/receive_message.rs:~423`). Stringly errors and `expect`s are unchanged |
| Domain types | 0 | `queue_key` still returns `(String, String)` (`state/addressing.rs:24`); `Queue::integer` byte-identical (`state/queue.rs:489`). The `Request`/`Taken` types are control-flow helpers, not domain types |
| Ownership | 0 | `queue.policy().map(str::to_string)` is kept (`ops/receive_message.rs:714`, `send_message.rs:344`). The new `deliver` takes `Option<&String>` (`ops/receive_message.rs:588`) |
| Comments | +1 | f45ada8d0/00bab2914 delete all `// ----` banners (metrics 0.61 -> 0); stranded rationale becomes module docs (`state/refusals.rs:5-14`, `state/limits.rs:3-10`); comment share 43.4% -> 42.9%. 21 new `//!` headers add volume; the em dashes and UPPERCASE stay |
| Control flow | +1 | a41704580 cuts receive `handle` from ~395 to 43 lines (`ops/receive_message.rs:302-345`). 5048c7b0a shrinks send `handle` 352 -> 206 and 63a315d74 shrinks batch 347 -> 219. create_queue (215) and start_message_move_task (259) are untouched |
| Lint suppressions | +1 | 586edbacb removes 3 crate-level `#![allow]` lines (6 lints) from `lib.rs:3-5`; `allow_attributes_without_reason` 196 -> 190 in clippy.txt. The op-file `#[allow(unused_imports)]` stay |
| Tests | 0 | no test files changed; 280/280 pass as in baseline |
| Commit hygiene | +2 | 24 small commits, each message states why; moves are separate from rewrites. Minor: a41704580 bundles the dead-check removal and comment rewording with the split (disclosed in the message) |
| Behavior risk | 0 | dropped check in a41704580 is provably dead (same input, same `message_token_refusal`, earlier return); parse-before-store order kept; extractions in 5048c7b0a/63a315d74 keep check order |

## Audit findings
| id | status | evidence |
|---|---|---|
| S-01 | untouched | |
| S-05 | untouched | |
| S-07 | untouched | |
| S-08 | untouched | |
| S-09 | untouched | |
| S-12/S-13 | untouched | |
| S-14/S-15 | partial | a41704580 removes the duplicate ReceiveRequestAttemptId check and 3 of 4 `usize::try_from(..).unwrap_or(1)` (`ops/receive_message.rs:446`) |
| S-16/S-17 | untouched | |
| S-18 | partial | a41704580 splits the audit exemplar `ReceiveMessage::handle` into parse/delivery_pending/prepare_delivery/take_messages |
| S-19 | partial | banners removed, rationale consolidated (00bab2914). The exemplar narration is still there (`state/queue.rs:340-341`) |
| S-20 | untouched | |
| S-21 | untouched | |

## Best hunks
a41704580, `ops/receive_message.rs:312-326`. The 400-line poll loop now reads as its algorithm, with the lock held only inside synchronous helpers:
```rust
let started = SystemTime::now();
loop {
    let now = SystemTime::now();
    let delivering = request.delivery_pending(&store, now);
    if delivering {
        self.prepare_delivery(&store, &request, ctx, now)?;
    }
    let (messages, wait) = match self.take_messages(&store, &request, ctx, now, delivering)? {
        Taken::Replayed(messages) => {
            return Ok(ReceiveMessageOutput { messages: Some(messages) });
        }
        Taken::Polled { messages, wait } => (messages, wait),
    };
```
5108af4fd, `ops/start_message_move_task.rs:217-227`. Two copies of the ARN classification collapse into one helper, so the two members' refusal wording cannot drift:
```rust
let source_name = queue_named_by(&input.source_arn, &region, "SourceArn")?;
let named_destination = input
    .destination_arn
    .as_deref()
    .map(str::trim)
    .filter(|arn| !arn.is_empty())
    .map(|arn| queue_named_by(arn, &region, "DestinationArn"))
    .transpose()?;
```

## Worst hunks
5048c7b0a, `ops/send_message.rs:355-366`. `prepare` hands back a 10-field bag that is immediately destructured, so the extraction moves 150 lines without naming any concept:
```rust
let Prepared {
    message_system_attributes, group_id, deduplication_id, delay,
    message_attributes, md5_of_message_attributes, size, retained,
    md5_of_body, md5_of_message_system_attributes,
} = prepare(queue, &input, fanned_out_by_the_notification_service)?;
```
state.rs, `state.rs:208-225`. 18 glob re-exports keep every item of every new module on the flat `sqs::` path, so the module boundaries do nothing to narrow visibility:
```rust
pub use addressing::*;
pub use attribute_validation::*;
...
pub use tags::*;
```

## Silent edits check
For every move commit I compared the multiset of added and removed lines, whitespace-trimmed. The only non-matching lines are `use` lists, `mod`/`pub use` declarations, `pub(super)` widenings (11), new `//!` module headers, dropped banners, and one doc line re-pointed from `ComAmazonawsSqsAmazonsqsState` to `super::...`. I also diffed these function bodies against baseline `state.rs`: `Queue::integer`, `Queue::hand_over`, `delivery_order`, `dead_letter_due`, `queue_key` and `check_attribute_value`. All are identical. In the rewrite commits, the bodies of `prepare` (send), `check_request_shape`, `refuse_parameters_the_queue_type_does_not_take` and `measure_entries` match the removed code line for line. The receive steps match too, apart from `request.` field access, the precomputed `max_messages`, `<Self as Operation>::NAME`, and em dashes reworded in comments.
