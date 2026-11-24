# p14r / simplicity

## Method

- Read every file under `BASE/src` in full to know the starting point and its existing simplicity debts: the `ContentBasedDeduplication` check copied in `create_queue` and `set_queue_attributes`, the dead-letter target check copied in the same two handlers, 11 `expect("required")` calls right after `require`, the `if let Some(message)` branch for a state that cannot occur in `change_message_visibility`, and handlers that look the same queue up twice.
- For each of the 15 attempts, ran `diff -ruN BASE/src <CODE>/src` and read the whole diff. The `tests/` directories are the same as BASE in every attempt, so only `src/` differs.
- Ran the same grep sweep on all 16 crates: `#[allow]`, debug output, `todo!`, the two copied checks (by error message text), the `expect("required")` style, new `required_*` getters, counter methods (`issue`/`take_next`/`take`/`next`), and new one-call-site helpers, enums and structs.
- Scored the whole crate. Each attempt is judged mainly on what it added or removed compared with BASE: duplicate decisions it removed, abstractions with a single user it added, blocks it copied, and whether the module layout still fits a crate of about 1300 lines.
- Did not run cargo. Read nothing outside `v2/blind/p14r/` and the spec.

## Scores

| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| MO | 3 | 1 | MO/src/attributes.rs:126 | MO/src/request.rs:31 | Smallest diff; puts the copied CBD check in one place and adds nothing else besides a Command enum, whose name list now also appears in `from_name` |
| ME | 3 | 2 | ME/src/broker/queue_commands.rs:18 | ME/src/ids.rs:35 | Shared CBD check and plain id types; small lapses are a copied `take_next`, `Ok(x?)` at queue_commands.rs:30 and `MAX_TICK = MAX_RETENTION` tying two unrelated limits together |
| KO | 2 | 3 | KO/src/attributes.rs:136 | KO/src/broker/message_commands.rs:53 | Shares the CBD check, but adds a `FifoIds` struct plus a `FifoSend` enum plus a helper for a single call site |
| PE | 2 | 4 | PE/src/broker/queue_commands.rs:20 | PE/src/broker/message_commands.rs:198 | Drops the `queue_name` wrapper and shares the CBD check; the new dead-letter helper looks the source queue up again, and `take_next` is copied 3 times (ids.rs:13/34/64) |
| JU | 2 | 5 | JU/src/broker/queue_commands.rs:19 | JU/src/queue.rs:184 | Shared CBD check and one `limits` table; splitting `push`/`push_fifo` makes `push_dead_letter` branch again and adds a `debug_assert` (queue.rs:151) |
| XE | 2 | 6 | XE/src/queue.rs:233 | XE/src/broker/message_commands.rs:203 | Removes the double lookups and the branch for a state that cannot occur, but copies the handle-parsing preamble into two handlers and adds single-use `Message::new` and `forget_dead_letter`; both BASE check copies remain |
| TA | 2 | 7 | TA/src/broker/queue_commands.rs:23 | TA/src/ids.rs:64 | The only attempt that also puts the dead-letter check in one place, but it adds a `time` module, a macro for two types (Arrival still repeats `take`), a `QueueName` with `Borrow`, and `apply` taking `dead_letter` while ignoring the same fields (attributes.rs:83) |
| CE | 2 | 8 | CE/src/queue.rs:191 | CE/src/request.rs:73 | Removes the branch for a state that cannot occur, but keeps the command as a String and parses it in a second step; both copied checks remain (queue_commands.rs:44/97) |
| BASE | 2 | 9 | BASE/src/request.rs:69 | BASE/src/broker/queue_commands.rs:97 | Compact module layout, but two copied validation blocks, 11 `expect` calls after `require`, and a branch for a state that cannot occur (message_commands.rs:196) |
| KA | 2 | 10 | KA/src/error.rs:55 | KA/src/broker/message_commands.rs:21 | Small change; moves the FIFO checks into a single-use helper and adds a `usize::try_from` + `expect` (message_commands.rs:132); BASE's copies remain |
| LA | 2 | 11 | LA/src/error.rs:58 | LA/src/broker/message_commands.rs:57 | Error constructors in one place; adds a single-use `FifoSend::New(Option<(..)>)` enum; both copied checks remain |
| VI | 2 | 11 | VI/src/error.rs:55 | VI/src/broker/message_commands.rs:52 | Nearly the same diff as LA: shared error constructors, the same single-use `FifoSend` with `New(Option)`, copies remain |
| VO | 2 | 13 | VO/src/error.rs:52 | VO/src/ids.rs:67 | Single-use `fifo_ids` helper and the same `issue` body copied into 3 id types; BASE's copies remain |
| DI | 2 | 14 | DI/src/error.rs:58 | DI/src/ids.rs:64 | Shared error constructors, but `issue` copied 3 times, a single-use `FifoIds::resolve`, and new `expect` branches around the dead-letter move (message_commands.rs:163) |
| DE | 1 | 15 | DE/src/request.rs:31 | DE/src/ids.rs:19 | Adds two identical allocator structs (`MessageIds`, `ReceiptHandles`) besides the id types, plus a single-use `SendRules` enum (message_commands.rs:36); both BASE copies remain |
| RU | 1 | 16 | RU/src/broker/message_commands.rs:196 | RU/src/broker/message_commands.rs:40 | The same `expect("presence checked by Params::require")` text appears at 11 call sites instead of one getter, `take_next` is copied 3 times, and both BASE copies remain |

## What separates the top from the bottom

- Putting a copied decision in one place without adding layers. MO, ME, KO, PE, JU and TA each turn the copied `ContentBasedDeduplication` check into one function (e.g. MO/src/attributes.rs:126). The 9 others keep it in both handlers (e.g. VI/src/broker/queue_commands.rs:40 and :90). Only TA also merges the dead-letter target check (TA/src/broker/queue_commands.rs:23).
- Diff size against payoff. MO and ME change little and remove a duplication. TA has the largest diff (733 changed lines) and removes the most duplication, but adds a `time` module, a macro and a `QueueName` newtype with a `Borrow` impl, which is a lot of machinery for a crate of about 1300 lines.
- Single-use abstractions. Almost every attempt moves the FIFO part of `SendMessage` into a new enum or struct with one caller: KO/src/broker/message_commands.rs:53, DE/src/broker/message_commands.rs:36, LA/src/broker/message_commands.rs:53, TA/src/broker/message_commands.rs:34. The ones that do this without removing any other duplication end up below BASE.
- Copied counter code. DI, PE, RU and VO copy the same 4-line `issue`/`take_next` body into three types (e.g. PE/src/ids.rs:13/34/64). DE adds whole allocator structs (DE/src/ids.rs:19). JU and XE use a `next(self)` value method instead, which is shorter but is still repeated.
- Copied text in place of a helper. RU repeats the same `expect` message 11 times (RU/src/broker/message_commands.rs:40 onward). XE removes `in_flight_message` and pastes its preamble into two handlers (XE/src/broker/message_commands.rs:187 and :203).

## Patterns seen in most crates

- 11 of 15 attempts replace the string dispatch with a `Command` enum (KA, KO, LA and VI do not). For simplicity this is neutral: the 11 names now appear in two matches.
- 8 attempts (CE, DE, DI, JU, ME, PE, TA, XE) add `required_string`/`required_integer` called right after `require`, so a missing parameter is still checked twice. This replaces BASE's `expect("required")` calls rather than removing the double check.
- No crate has `#[allow]`, debug output, `todo!` or dead code. The `broker/` submodule split from BASE stays in every attempt.
- The dead-letter target check stays in two handlers everywhere except TA, and the double queue lookup in `set_queue_attributes` stays everywhere (e.g. BASE/src/broker/queue_commands.rs:87).
