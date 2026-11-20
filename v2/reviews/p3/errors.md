# p3 / errors

## Method

All 19 crates were read the same way, and nothing outside `v2/blind/p3/` and `task/SPEC.md` was opened.

1. I read every file in BASE `src/` in full to find its error-handling weak points.
2. For every attempt I ran `diff -r -U2 BASE/src <CODE>/src` and read each hunk that touches failure paths: `Result`, `Option`, `expect`, casts, arithmetic and error construction.
3. I ran one grep over every crate's `src/`: `unwrap(`, `expect(`, `unreachable!`, `panic!`, `as u64|usize|...`, `unwrap_or_default`, `.ok()`, `let _ =`, `Result<_, String>`, and `checked_/saturating_/wrapping`.
4. In every crate I looked up the same five sites by line:
   - the checked integer conversion in `request.rs`;
   - the `ChangeMessageVisibility` lookup (`message_mut(key)`);
   - the `max_messages as usize` cast;
   - the `.expect("required")` reads after `require`;
   - the dead-letter hand-off in `receive_message`.

I did not run cargo, and I did not read `tests/`, because this dimension is about production code paths.

BASE is already decent:
- the error type is typed (`ErrorCode` enum plus `Error`);
- no `Result` is dropped;
- integers are parsed with `u64::try_from` and a range check.

Its lapses:
- about 12 `.expect("required")` calls whose invariant depends on a separate `require` call;
- a silent `if let Some` in `ChangeMessageVisibility` for a state that cannot occur;
- a silent drop of dead letters when the target queue is missing;
- one `as usize` cast;
- unchecked `now + x` arithmetic.

Attempts are scored on which of these lapses they removed or added.

## Scores

| code | score | rank | best evidence | worst evidence | reason |
|---|---|---|---|---|---|
| QO | 4 | 1 | QO/src/broker/message_commands.rs:177 | QO/src/broker/message_commands.rs:161 | `required_*` getters replace every `expect("required")`; handle lookup returns `Result` (`in_flight_mut(..).ok_or_else(handle_expired)`), no new expect; cast gone; only the shared silent dead-letter drop remains |
| CU | 4 | 2 | CU/src/request.rs:161 | CU/src/time.rs:34 | `require` returns the present params, so no "required" expects can exist; silent visibility branch becomes a named expect; cast gone; adds an unchecked `Instant::since` subtraction and one more invariant expect (message_commands.rs:108) |
| ZA | 4 | 2 | ZA/src/broker/message_commands.rs:197 | ZA/src/broker/message_commands.rs:165 | all "required" expects gone through `required_string`/`required_integer`; silent `if let` becomes a named expect; cast removed; shared dead-letter drop remains |
| FE | 3 | 4 | FE/src/request.rs:78 | FE/src/broker/message_commands.rs:149 | expects replaced and the visibility branch fixed (queue.rs:191 named expect), but `as usize` on input stays |
| ME | 3 | 5 | ME/src/request.rs:82 | ME/src/broker/message_commands.rs:191 | expects and cast removed; the silent `if let Some(message)` for an impossible state stays |
| SI | 3 | 5 | SI/src/request.rs:168 | SI/src/broker/message_commands.rs:195 | `require` returns params (a hidden `NULL` placeholder never escapes); cast removed; silent visibility branch stays |
| TE | 3 | 5 | TE/src/request.rs:88 | TE/src/broker/message_commands.rs:190 | expects and cast removed; silent visibility branch stays |
| PI | 3 | 8 | PI/src/request.rs:78 | PI/src/broker/message_commands.rs:190 | expects removed; cast (line 149) and silent branch stay |
| BASE | 2 | 9 | BASE/src/request.rs:100 | BASE/src/broker/message_commands.rs:196 | typed errors and checked parsing, but about 12 `expect("required")` calls, a silent impossible branch, `as usize`, unchecked time arithmetic |
| FI | 2 | 9 | FI/src/request.rs:94 | FI/src/broker/message_commands.rs:228 | refactor adds error constructors only; every BASE lapse kept |
| FO | 2 | 9 | FO/src/request.rs:97 | FO/src/broker/message_commands.rs:198 | constructor and constant extraction only; BASE lapses kept |
| HO | 2 | 9 | HO/src/request.rs:97 | HO/src/broker/message_commands.rs:193 | same as BASE on this dimension |
| JI | 2 | 9 | JI/src/request.rs:97 | JI/src/broker/message_commands.rs:193 | same as BASE on this dimension |
| NU | 2 | 9 | NU/src/request.rs:94 | NU/src/broker/message_commands.rs:214 | FIFO checks extracted with order kept; BASE lapses kept |
| PO | 2 | 9 | PO/src/request.rs:94 | PO/src/broker/message_commands.rs:216 | same as NU |
| SU | 2 | 9 | SU/src/request.rs:102 | SU/src/broker/message_commands.rs:226 | FIFO helper only; BASE lapses kept |
| VA | 2 | 9 | VA/src/request.rs:97 | VA/src/broker/message_commands.rs:193 | constants and constructor only |
| VO | 2 | 9 | VO/src/request.rs:97 | VO/src/broker/message_commands.rs:237 | named error constructors; BASE lapses kept |
| WO | 2 | 9 | WO/src/request.rs:102 | WO/src/broker/message_commands.rs:193 | constants only |

## What separates the top from the bottom

- **Required parameters.** The top eight make "present" part of the read: `required_string` (ZA/src/request.rs:83, QO/src/request.rs:120), or `require` returning the values (CU/src/request.rs:161, SI/src/request.rs:168). The bottom eleven keep `params.string(..)?.expect("required")` at each call site (for example WO/src/broker/message_commands.rs:54). That expect is only safe while the separate `require` list stays in sync.
- **States that cannot occur.** Only QO, CU, ZA and FE remove the silent `if let Some(message) = ...message_mut(key)` (BASE/src/broker/message_commands.rs:196):
  - QO turns it into a typed error path (QO/src/broker/message_commands.rs:177);
  - CU, ZA and FE turn it into an `expect` that names its invariant (CU/src/broker/message_commands.rs:185, ZA/src/broker/message_commands.rs:198, FE/src/queue.rs:191).
- **Casts on input.** CU, ME, QO, SI, TE and ZA pass `max_messages` as `u64` and drop the cast, so `plan_receive` takes the type of the parameter (ZA/src/queue.rs:194). The rest keep `max_messages as usize` (VO/src/broker/message_commands.rs:194). Range checking makes it harmless, but it is still an `as` on input.
- **No regressions.** No attempt introduced a dropped `Result`, a `String` error type, a failure turned into a default, or a change in check order. The bottom group is penalized for leaving BASE as it was, not for making it worse.

## Patterns seen in most crates

- Every crate keeps the typed `ErrorCode` + `Error` pair, checked `u64::try_from` + range parsing, and receipt-handle parsing through `Option` with no panics.
- Every crate keeps two shared lapses:
  - unchecked `now + delay` / `now + timeout` / `sent_at + retention` arithmetic (in practice bounded by `MAX_TICK` and attribute ranges);
  - silent dropping of dead-lettered messages when the target queue is missing, behind `if let Some(target) = ...get_mut` (for example BASE/src/broker/message_commands.rs:168 and QO/src/broker/message_commands.rs:161).
- Every crate keeps `expect("planned message exists")` in `Queue::deliver`, which names its invariant.
- Most "cleanups" (11 of 18) aimed at duplication: shared error constructors, range constants and FIFO helpers. They left error handling unchanged.
