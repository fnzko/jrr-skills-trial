# p14r / errors

## Method
BASE was read in full. Then, for all 16 crates, the same steps:
1. One grep over `src/` for `unwrap(`, `expect(`, `unreachable!`, `panic!`, `as u64/usize/u32/i64`, `let _ =`, `.ok()`, `unwrap_or_default`, `checked_`, `saturating_`.
2. One grep for the error lapses found in BASE: `message_mut`, `and_then(... get_mut)`, `.remove(key);`, `now - `, `dead_letters`, `require`/`required_*`.
3. `diff BASE/src/error.rs` and the `receive_message` .. end of `broker/message_commands.rs` (receive, delete, change visibility, handle resolution), read in every crate.
4. The FIFO `SequenceNumber` handling in `send_message` and the `plan_receive` signature (to see whether the `as usize` cast is still there), in every crate.
5. A grep over the `diff -r BASE/src <CODE>/src` added lines for `unwrap_or`, `.ok(`, `if let`, `let Some`, `expect`, `debug_assert`, `return;`, `_ =>`.
Test modules are not counted. I did not run cargo.

BASE baseline: the error codes are typed (`ErrorCode` enum), input goes through checked conversion (`u64::try_from`), and no input can cause a panic. It has five recurring lapses:
- L1: 11 x `expect("required")`, an expect that does not name its invariant.
- L2: a silent `if let Some(message) = ...message_mut(key)` in ChangeMessageVisibility. This is a branch for a state that cannot occur.
- L3: silent dead-letter drop through `dead_letter_queue.and_then(|q| self.queues.get_mut(&q))`.
- L4: silent `if let (Some(..), Some(sequence_number))` on a FIFO send.
- L5: the `max_messages as usize` cast.
Scores mostly count which of these each attempt removed, and how.

## Scores
| code | score | rank | best evidence | worst evidence | reason |
|---|---|---|---|---|---|
| JU | 4 | 1 | JU/src/queue.rs:157 (`push_fifo` returns `u64`, no Option to skip) | JU/src/broker/message_commands.rs:184 (silent dead-letter drop kept) | Fixes L1, L2, L4 (structurally), L5 |
| RU | 4 | 1 | RU/src/broker/message_commands.rs:157 (DLQ lookup `expect` names the DeleteQueue invariant) | RU/src/broker/message_commands.rs:40 (11 required-param expects kept, now named) | Fixes L2, L3, L5, names every remaining expect |
| TA | 4 | 1 | TA/src/broker/message_commands.rs:138 (named expect replaces silent FIFO skip) | TA/src/broker/message_commands.rs:187 (silent dead-letter drop kept) | Fixes L1, L2, L4, L5 |
| XE | 4 | 1 | XE/src/broker/message_commands.rs:205 (`in_flight_mut(..).ok_or_else(expired_handle)`: impossible branch gone by construction) | XE/src/broker/message_commands.rs:171 (silent dead-letter drop kept) | Fixes L1, L2 structurally, L4, L5 |
| CE | 3 | 5 | CE/src/queue.rs:194 (named expect replaces silent `if let`) | CE/src/broker/message_commands.rs:148 (silent dead-letter drop) | Fixes L1, L2, L5; L3 and L4 kept |
| DI | 3 | 5 | DI/src/broker/message_commands.rs:171 (DLQ expect names the DeleteQueue invariant) | DI/src/broker/message_commands.rs:197 (silent change-visibility `if let`) | Fixes L1, L3, L5; L2 and L4 kept |
| DE | 2 | 7 | DE/src/request.rs:131 (`required_string` returns `MissingParameter` rather than expect) | DE/src/broker/message_commands.rs:206 (silent `if let`) | Fixes only L1 |
| KA | 2 | 7 | KA/src/broker/message_commands.rs:132 (`usize::try_from` with a named expect) | KA/src/broker/message_commands.rs:191 (silent `if let`) | Fixes only L5; 11 vague expects kept |
| ME | 2 | 7 | ME/src/request.rs:120 (`required_string`) | ME/src/broker/message_commands.rs:173 (silent `if let`) | Fixes only L1 |
| PE | 2 | 7 | PE/src/request.rs:119 (`required_string`) | PE/src/broker/message_commands.rs:206 (helper documents and keeps a silent message drop) | Fixes L1; turns L3 into a documented contract |
| BASE | 1 | 11 | BASE/src/request.rs:100 (`u64::try_from` checked conversion) | BASE/src/broker/message_commands.rs:196 (silent `if let` for an impossible state) | All five lapses |
| KO | 1 | 11 | KO/src/request.rs:102 (checked conversion) | KO/src/broker/message_commands.rs:92 (`expect("required")` repeated) | Error handling unchanged from BASE |
| LA | 1 | 11 | LA/src/request.rs:94 (checked conversion) | LA/src/broker/message_commands.rs:228 (silent `if let`) | Unchanged from BASE apart from constructor helpers |
| MO | 1 | 11 | MO/src/request.rs:137 (checked conversion) | MO/src/broker/message_commands.rs:192 (silent `if let`) | Unchanged from BASE |
| VI | 1 | 11 | VI/src/request.rs:94 (checked conversion) | VI/src/broker/message_commands.rs:216 (silent `if let`) | Unchanged from BASE apart from constructor helpers |
| VO | 1 | 11 | VO/src/request.rs:137 (checked conversion) | VO/src/broker/message_commands.rs:188 (silent `if let`) | Unchanged from BASE |

## What separates the top from the bottom
- Impossible states. The top crates either remove them from the types or panic with a named invariant. XE/src/queue.rs:233 `in_flight_mut` returns the message itself, so the separate "key not found" branch is gone. JU/src/queue.rs:157 `push_fifo` returns a plain sequence number. The bottom crates keep the BASE `if let` that silently does nothing (VO/src/broker/message_commands.rs:188).
- Required parameters. Most of the top crates read them through a `Result`-returning getter (`required_string`, TA/src/request.rs:122). The bottom crates keep 11 `expect("required")` that name no invariant (KO/src/broker/message_commands.rs:92).
- Dead-letter delivery. Only DI and RU state that the target queue must exist (DI/src/broker/message_commands.rs:171, RU/src/broker/message_commands.rs:157). Every other crate silently drops the messages if it does not. PE goes further and writes the silent drop into a helper's contract (PE/src/broker/message_commands.rs:198).
- Casts. CE, DI, JU, RU, TA and XE pass `max_messages` as `u64` to `plan_receive`. KA uses `try_from` with a named expect. The rest keep `as usize`, which is harmless here because the value is range-checked to 1..=10.

## Patterns seen in most crates
- All 16 keep the typed `ErrorCode` enum, check integer input with `u64::try_from` plus a range filter, and parse receipt handles exactly. Input cannot cause a panic in any crate.
- No crate drops a `Result`. The ignored `Option` from `queue.remove(key)` in DeleteMessage stays everywhere except XE, which uses `remove_in_flight(..).ok_or_else`.
- Clock arithmetic (`now + timeout`, `now - accepted_at`) is unchecked everywhere. The spec bounds time at 4e9 and `accepted_at <= now` always holds, so this is not counted.
- Most attempts add shorthand error constructors (`Error::missing_parameter`, `invalid_value`). They tidy the code but change nothing about error safety.
