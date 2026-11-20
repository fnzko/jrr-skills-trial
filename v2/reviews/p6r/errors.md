# p6r / errors

## Method
- Read BASE `src/` in full and listed its error-handling lapses. There are four: about 10 `params.string(..)?.expect("required")` after `require()`; the silent `if let Some(message) = ...message_mut(key)` in ChangeMessageVisibility (BASE/src/broker/message_commands.rs:196); dead letters dropped silently when the dead-letter queue lookup misses (`and_then(|q| self.queues.get_mut(&q))`, :168); and `max_messages as usize` (:153).
- For every attempt: ran `diff -ruN BASE/src <CODE>/src` and read the whole diff. Then ran the same grep in every crate for `expect(`, `.unwrap()`, `as u*/usize`, `try_from`, `unwrap_or`, `.ok()`, `let _`, `panic!/unreachable!`, `message_mut`, `get_mut`, plus the `required_*`/`require([..])` helpers. Then checked whether each of the four BASE lapses was fixed, kept or made worse. No attempt changed `tests/`, so none was read. No cargo was run.
- What every crate shares from BASE is typed errors (`ErrorCode` enum plus message), strict integer parsing via `as_i64` + `u64::try_from` + range, and no `unwrap` on input. That shared floor does not separate crates. The scores reflect what each attempt did to the four lapses.

## Scores
| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| JI | 4 | 1 | JI/src/request.rs:161 (`require` returns the present values, so there is no `expect("required")`) | JI/src/broker/message_commands.rs:156 (a missing DLQ still drops messages silently) | Removed the required-param panics structurally, removed the `as` cast, CMV uses a named-invariant expect, QueueKind makes "CBD on standard queue" unrepresentable; one silent path left |
| KU | 4 | 1 | KU/src/broker/message_commands.rs:160 (DLQ lookup is an expect that names the DeleteQueue invariant) | KU/src/broker/message_commands.rs:40 (10 repeated expects after `require`, all named but avoidable) | Fixed every BASE lapse (CMV, DLQ, the cast); every remaining expect states its invariant |
| NE | 4 | 1 | NE/src/broker/message_commands.rs:190 (`in_flight_mut(..).ok_or_else(handle_expired)`: Option becomes a typed error, not a silent branch) | NE/src/broker/message_commands.rs:161 (DLQ miss still silently drops) | Total `required_*` getters, `u64` max count (no cast), CMV/Delete go through Option-to-error; one silent path left |
| QO | 3 | 4 | QO/src/broker/message_commands.rs:207 (handle resolution on the single looked-up queue; CMV expect names its source, :198) | QO/src/broker/message_commands.rs:165 (DLQ silent drop) | required getters, cast removed, CMV made explicit |
| PA | 3 | 4 | PA/src/queue.rs:194 (`set_invisible_until` expect names `find_in_flight`) | PA/src/broker/message_commands.rs:148 (DLQ silent drop) | required getters, `plan_receive` takes u64 (no cast), CMV explicit; second queue lookup kept |
| FO | 3 | 4 | FO/src/broker/message_commands.rs:197 (returns `(&mut Queue, key)`, so there is no re-lookup that could miss) | FO/src/broker/message_commands.rs:149 (`max_messages as usize` kept) | required getters and CMV fixed; cast and DLQ drop kept |
| CE | 3 | 4 | CE/src/request.rs:168 (`require` returns typed params; no expect, no dead MissingParameter path) | CE/src/broker/message_commands.rs:195 (silent `if let Some(message)` kept) | Best fix for required params, cast removed, but both silent branches kept |
| PI | 3 | 4 | PI/src/broker/message_commands.rs:156 (DLQ expect names the DeleteQueue invariant) | PI/src/attributes.rs:87 (`unreachable!` for a `Partial` state the type still allows) | Every BASE lapse addressed with named expects, but adds a panic arm for a representable state |
| MI | 3 | 4 | MI/src/broker/message_commands.rs:153 (DLQ expect with reason) | MI/src/broker/message_commands.rs:37 (terse `expect("required")` kept ×9) | Fixed CMV, DLQ and the cast; left the required expects untouched |
| KE | 2 | 10 | KE/src/request.rs:78 (`required_string` returns Result) | KE/src/broker/message_commands.rs:190 (silent CMV branch kept) | Removed the required-param panics only; cast and both silent paths kept |
| TE | 2 | 10 | TE/src/broker/message_commands.rs:155 (DLQ silent drop turned into a commented, named expect) | TE/src/broker/message_commands.rs:183 (silent CMV branch kept) | One lapse fixed; required expects and cast kept |
| DE | 2 | 10 | DE/src/broker/message_commands.rs:132 (`usize::try_from` + named expect) | DE/src/broker/message_commands.rs:191 (silent CMV branch kept) | Only the cast fixed |
| PO | 2 | 10 | PO/src/broker/message_commands.rs:134 (`usize::try_from(..).expect("range is 1..=10")`) | PO/src/broker/message_commands.rs:193 (silent CMV branch kept) | Only the cast fixed |
| BASE | 1 | 14 | BASE/src/request.rs:100 (strict integer: `as_i64` + `u64::try_from` + range) | BASE/src/broker/message_commands.rs:196 (silent `if let Some` on an impossible miss) | Typed errors, but terse expects in every handler, two silent paths, an `as` cast |
| BE | 1 | 14 | BE/src/request.rs:31 (unknown command rejected at parse as InvalidRequest) | BE/src/broker/message_commands.rs:193 (silent CMV branch) | No change to any BASE error lapse |
| FI | 1 | 14 | FI/src/request.rs:31 (Command parsed at the boundary) | FI/src/broker/message_commands.rs:192 (silent CMV branch) | No change to any BASE error lapse |
| PE | 1 | 14 | PE/src/request.rs:102 (inherited strict integer check) | PE/src/broker/message_commands.rs:226 (silent CMV branch) | No change to any BASE error lapse |
| XE | 1 | 14 | XE/src/request.rs:94 (inherited strict integer check) | XE/src/broker/message_commands.rs:228 (silent CMV branch) | No change to any BASE error lapse |
| ZE | 1 | 14 | ZE/src/request.rs:94 (inherited strict integer check) | ZE/src/broker/message_commands.rs:216 (silent CMV branch) | No change to any BASE error lapse |

## What separates the top from the bottom
- How required parameters are read. The top crates make the read total. CE/JI `require` returns the present values (CE/src/request.rs:168, JI/src/request.rs:161). NE/QO/PA/FO/KE use `required_*` getters that return `Result`. KU and PI keep `expect`, but each one states the invariant (KU/src/broker/message_commands.rs:40). The bottom six keep BASE's bare `expect("required")` in every handler (e.g. ZE/src/broker/message_commands.rs:94).
- The impossible-miss branch in ChangeMessageVisibility. The top crates turn the silent `if let Some(message)` either into an expect that names its invariant (JI :185, KU :198, PA/src/queue.rs:194, QO :198, FO/src/queue.rs:191) or into Option -> `ReceiptHandleExpired` (NE :190). CE, KE, TE, DE, PO and the bottom six keep the silent branch.
- Silently dropped dead letters. Only KU, MI, PI and TE replaced `and_then(|q| self.queues.get_mut(&q))` with an expect that cites DeleteQueue (KU :160, TE :155). Everyone else, including JI and NE, still discards messages without a trace if the invariant ever breaks.
- The `as usize` cast. QO, PA, NE, CE and JI made `plan_receive` take `u64`, so the cast is gone. PO, DE, KU, MI and PI use `usize::try_from` with a named expect. BE, FI, PE, XE, ZE, TE, FO and KE keep `as usize`.
- One step backwards: PI adds `unreachable!` for `DeadLetterParams::Partial` (PI/src/attributes.rs:87), a state its own enum allows. CE and JI fold the same state away without a panic.

## Patterns seen in most crates
- All 19 keep BASE's typed error (`ErrorCode` enum, never a `String` error), strict integer parsing, and no `unwrap` reachable from input. No crate converts a request failure into a default. The remaining `unwrap_or` calls are spec defaults for absent optional parameters.
- About a third of the attempts (BE, FI, PE, XE, ZE, and mostly DE/PO/TE) worked on other dimensions (Command enum, id newtypes, constants, helper extraction) and left the error lapses as they were. Many added an `Error::missing_parameter`/`invalid_value` constructor, which reduces duplication but changes no error behavior.
- Nobody touched the unchecked `now - accepted_at` or `now + timeout` arithmetic. It is bounded by the spec's clock limit, but only JI documents the precondition (`Instant::since`).
