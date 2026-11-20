# Review of run 12
## Summary
Run 12 is a small, conservative attempt: 5 commits, 25 files, +117/-170, no code moved between files and no module restructuring.
It removes the inert `#[allow(unused_imports)]` above `use super::types::*;` in all 23 SQS op files, deletes two `EmulatedService` override stubs that restated the trait defaults, borrows the queue ARN and policy for `authorization::denied` in 12 handlers, moves four request fields instead of cloning them in SendMessage, and folds the push/deduplicate/count tail shared by SendMessage and SendMessageBatch into `Queue::enqueue_sent`.
It leaves everything structural alone: the stringly `(String, String)` queue keys and attribute map, the giant handlers, the per-op `lift`/`store_unavailable` boilerplate, the `expect` after prior lookups, the store-wide sweeps and the 50 ms long-poll loop, and the comment volume (43.4% -> 43.5% of src lines).
Tests are untouched and still pass (280/280), and clippy's total drops from 360 to 268 entirely through the removed allows.
## Commit classification
| hash | subject | kind | pure move? |
|---|---|---|---|
| b08fed989 | drop override stubs that restate the trait defaults | other (dead code removal) | no |
| 7c0c90203 | borrow the queue's ARN and policy for the authorization check | rewrite (ownership) | no |
| 46c69e1ba | move SendMessage's request fields instead of cloning them | rewrite (ownership) | no |
| 2891bf700 | drop allow(unused_imports) from the handlers' type glob | other (lint) | no |
| 8f2a3661c | give the send handlers one Queue::enqueue_sent | dedupe | no |
## Scores
| dimension | score | evidence |
|---|---|---|
| 1 Structure | 0 | No layout change; only a new `pub fn enqueue_sent` on `Queue` (8f2a3661c, src/state.rs:1553). |
| 2 Duplication | +1 | 8f2a3661c: the push/record_deduplication/count_sent tail of both send paths now lives once in src/state.rs:1553-1575; b08fed989 drops two stubs that duplicated trait defaults (src/service.rs:625, vendor/smithy-server-runtime/src/service.rs:390,405). Narrow: the 24-field `StoredMessage` literal is still repeated in src/ops/send_message.rs:554 and src/ops/send_message_batch.rs:543. |
| 3 Error handling | 0 | No `expect`, swallowed result or unreachable branch touched. |
| 4 Domain types | 0 | Untouched; the new API itself takes `deduplication_key: Option<(String, String)>` (src/state.rs:1556). |
| 5 Ownership | +1 | 7c0c90203: 12 handlers pass `&queue.arn, queue.policy()` instead of two owned copies (e.g. src/ops/delete_message.rs:215-216); 46c69e1ba: 4 clones of owned input removed (src/ops/send_message.rs:358,376,396,449). clone() 8.42 -> 7.52 per KLOC. |
| 6 Comments | 0 | Only the stub narration in b08fed989 went; comment share 43.4% -> 43.5%. The comment kept above `queue.enqueue_sent(` (src/ops/send_message.rs:551-552) now describes only the counting half of the call. |
| 7 Control flow | 0 | No handler shortened or split; `enqueue_sent` uses a `zip().map()` where an `if let` was (src/state.rs:1560-1569), neutral. |
| 8 Lint suppressions | +1 | 2891bf700 removes 23 `#[allow(unused_imports)]` (e.g. src/ops/add_permission.rs:108); clippy allow_attributes 142 -> 96, allow_attributes_without_reason 196 -> 150. The 24 in generated src/ops/types/ remain. |
| 9 Tests | 0 | No test changed. |
| 10 Commit hygiene | +1 | Five small commits, one kind each, bodies explain why and in what scope ("Handlers that release the lock before authorizing keep their copies"). 8f2a3661c's body overstates: message_id and sequence_number are still cloned at the call site (src/ops/send_message.rs:555,562) and again inside `enqueue_sent` (src/state.rs:1560,1563). |
| 11 Behavior risk | 0 | `enqueue_sent` keeps the order push -> record_deduplication -> count_sent and the same FIFO-only condition; stub removal matches the vendored defaults exactly; borrows happen under the same lock. |
## Audit findings
| id | status | evidence |
|---|---|---|
| S-01 | untouched | |
| S-05 | untouched | |
| S-07 | untouched | |
| S-08 | untouched | |
| S-09 | untouched | |
| S-12/S-13 | partial | 7c0c90203 removes 12 of 18 `policy().map(str::to_string)` copies; 6 remain (src/ops/untag_queue.rs:201 etc.); `queue_key` allocations and `expect` after lookup untouched. |
| S-14/S-15 | untouched | |
| S-16/S-17 | partial | S-17: 2891bf700 removes the outer allow in all 23 op files; S-16 `lift`/`store_unavailable` boilerplate untouched. |
| S-18 | untouched | |
| S-19 | untouched | |
| S-20 | untouched | |
| S-21 | untouched | (STS seam, not in this crate) |
## Best hunks
7c0c90203, src/ops/delete_message.rs:211-217 (same shape in 11 other handlers)
```rust
-        let (queue_arn, policy) = (queue.arn.clone(), queue.policy().map(str::to_string));
         if let Some(denied) = sqs::authorization::denied(
             &*self.engine,
             ctx,
             Self::NAME,
-            &queue_arn,
-            policy.as_deref(),
+            &queue.arn,
+            queue.policy(),
         ) {
```
Removes two allocations and two temporaries per request, and the commit correctly leaves the handlers that drop the lock first.

8f2a3661c, src/state.rs:1553-1575
```rust
    pub fn enqueue_sent(&mut self, message: StoredMessage,
        deduplication_key: Option<(String, String)>, size: usize, now: SystemTime) {
        let admitted = deduplication_key.zip(message.sequence_number.clone()).map(
            |(key, sequence_number)| { /* Deduplication { message_id, sequence_number, accepted_at: now } */ });
        self.messages.push_back(message);
        if let Some((key, record)) = admitted {
            self.record_deduplication(key, record);
        }
        self.count_sent(size, now);
    }
```
The only real dedupe: the two send paths can no longer drift on what accepting a message owes the queue.
## Worst hunks
8f2a3661c, src/ops/send_message.rs:551-558
```rust
        // One message was added to this queue, of the size Amazon SQS measures a message at:
        // what the `AWS/SQS` sent counters count (`sqs::QueueTraffic`).
        queue.enqueue_sent(
            sqs::StoredMessage {
                message_id: message_id.clone(),
                ...
                sequence_number: sequence_number.clone(),
```
The comment now heads a call that also appends and deduplicates, and the clones the commit message says it removed are still here (and repeated inside `enqueue_sent`).
## Silent edits check
No commit moves code between files, so there are no moved bodies to diff. The one relocated logic block, the send tail now in `Queue::enqueue_sent`, was compared against baseline src/ops/send_message.rs and src/ops/send_message_batch.rs: same statements in the same order (push_back, then `record_deduplication` only when both key and sequence number exist, then `count_sent(size, now)`), with the `Deduplication` built from the same values. The removed `override_routing_error`/`override_framework_error` bodies match the vendored trait defaults (vendor/smithy-server-runtime/src/service.rs:390-411). No silent edits found.
