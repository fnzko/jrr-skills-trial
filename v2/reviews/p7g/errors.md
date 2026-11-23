# p7g / errors

## Method
Same steps for all 21 crates, with no `BASE` to diff against. I read `src/error.rs` and `src/main.rs` in full, plus the stdin/stdout loop in each crate (lib/serve/session/server). For every `src/**/*.rs`, I cut each file at its first `#[cfg(test)]`, skipped `tests.rs` files, and ran the same greps on what remained:
`.unwrap()`, `.expect(`, `panic!/unreachable!`, `as u*/i*` casts, `.ok()`, `unwrap_or*`, `saturating_/checked_`, slice indexing, bare `+ - += -=`, `let _ =`, `serde_json::to_*`, the integer-parsing helper (`as_i64/as_u64/Number`), and dead-letter or `if let Some(x) = map.get_mut(..)` branches. I opened the surrounding lines for every hit that was not plainly an `Option` default (absent attribute or parameter -> spec default). `tests/` was not read. I did not run cargo.

Clean in every crate: no `.unwrap()`, no `panic!`, no `let _ =`, all I/O errors go up with `?` to a non-zero exit, and no error type is `Result<_, String>`.

## Scores
| code | score | rank | best evidence | worst evidence | reason |
|---|---|---|---|---|---|
| BO | 4 | 1 | BO/src/error.rs:52 | BO/src/types.rs:67 | Most structured error enum (a variant per code with typed sub-reasons); every `expect` names its invariant; only bare `+` is a clock add the spec bounds |
| BU | 4 | 1 | BU/src/handles.rs:34 | BU/src/types.rs:45 | Handle lookup uses `checked_sub` + `try_from`; `expect`s name invariants; nothing input-driven panics |
| FA | 4 | 1 | FA/src/request.rs:144 | FA/src/broker.rs:58 | Integer input via `as_u64` + range check; conversions after the range check use `expect` with a reason |
| FO | 4 | 1 | FO/src/store.rs:31 | FO/src/queue.rs:382 | Checked handle parsing; dead-letter target missing is an `expect` naming the DeleteQueue invariant |
| JU | 4 | 1 | JU/src/request.rs:282 | JU/src/broker.rs:100 | Generic `T::try_from` on every integer parameter; one named `expect` |
| KE | 4 | 1 | KE/src/command.rs:265 | KE/src/queue.rs:345 | `Bounded::try_from` on input; `expect`s all name an invariant (one message is repeated) |
| RO | 4 | 1 | RO/src/params.rs:39 | RO/src/queue.rs:255 | Explicit `is_u64/is_i64` check, then checked `u32::try_from`; invariant `expect`s only |
| VU | 4 | 1 | VU/src/request.rs:160 | VU/src/broker.rs:320 | Typed bounded parameters; the one `expect` is the dead-letter invariant |
| LE | 3 | 9 | LE/src/params.rs:51 | LE/src/queue.rs:419 | Clean parsing, but ChangeMessageVisibility silently skips when the message it just validated is missing |
| LO | 3 | 9 | LO/src/request.rs:301 | LO/src/broker.rs:361 | Parsing is sound, but receive handling chains five `expect`s plus raw `messages()[index]` indexing (queue.rs:509) |
| MO | 3 | 9 | MO/src/protocol.rs:148 | MO/src/queue.rs:336 | `usize::try_from(max).unwrap_or(usize::MAX)` and `u32` clamping (types.rs:27) stand in for invariant `expect`s |
| RA | 3 | 9 | RA/src/request.rs:215 | RA/src/request.rs:138 | `bounded(..)? as usize` is an unchecked `as` (safe only because of the range); everything else is checked |
| SU | 3 | 9 | SU/src/request.rs:107 | SU/src/queue.rs:441 | Silent `if let Some` on a message the handle check just found |
| VA | 3 | 9 | VA/src/params.rs:91 | VA/src/broker.rs:339 | Oversized integers saturate deliberately; a missing dead-letter queue silently drops the moved messages instead of asserting |
| XI | 3 | 9 | XI/src/types.rs:157 | XI/src/broker.rs:312 | A "cannot happen" missing dead-letter queue returns `?` after the receive has already changed state, so a failed command would change state |
| ZE | 3 | 9 | ZE/src/json.rs:23 | ZE/src/service.rs:310 | Same as XI: `?` on the dead-letter lookup after messages were already removed |
| ZU | 3 | 9 | ZU/src/request.rs:186 | ZU/src/queue/mod.rs:361 | `set_state` silently ignores a missing slot; otherwise clean |
| MA | 2 | 18 | MA/src/params.rs:98 | MA/src/broker.rs:328 | Dead-letter messages dropped if the target is missing; silent skips at queue.rs:357 and 404; `len() as u64 + 1` (broker.rs:308); `try_from(..).unwrap_or(1)` (handler.rs:247); `.ok()` turns MaxReceiveCount into None (attributes.rs:46) |
| NA | 2 | 18 | NA/src/command.rs:247 | NA/src/broker.rs:271 | `let Ok(target) = lookup(..)` swallows a real `Result` and drops dead-lettered messages; silent skip at queue.rs:418; `n as usize` (command.rs:161) |
| QE | 1 | 20 | QE/src/message_commands.rs:177 | QE/src/message_commands.rs:148 | Required params re-read as `Option` and `.expect("required")` about 12 times; 9 `as` casts; `handles[number as usize - 1]`; `unwrap_or(0)` on required Seconds (broker.rs:90); silent dead-letter drop (104) |
| WI | 1 | 20 | WI/src/request.rs:101 | WI/src/broker/message_commands.rs:168 | Same `.expect("required")` pattern about 11 times; `max_messages as usize`; dead-letter messages silently dropped; silent `if let Some(message)` (196) |

## What separates the top from the bottom
- Required parameters: top crates parse into typed commands where a required value is not an `Option` (BO/src/parse.rs:181, `required_string`). QE and WI check `require(...)`, then fetch the same value again as `Option` and `.expect("required")` it (QE/src/message_commands.rs:19, WI/src/broker.rs:86). If the two lists ever drift apart, user input panics the process.
- Impossible dead-letter target: top crates `expect` with the DeleteQueue invariant (FO/src/store.rs:149, RO/src/service.rs:247). MA, NA, QE, VA and WI wrap it in `if let`/`let Ok`, so a broken invariant loses messages without a trace (NA/src/broker.rs:271). XI and ZE turn it into `?` after mutating state (XI/src/broker.rs:312).
- Numeric conversion: top crates use `try_from` on the parsed integer (JU/src/request.rs:282, KE/src/command.rs:265). The bottom ones range-check, then cast with `as` (QE/src/queue.rs:95, RA/src/request.rs:138, NA/src/command.rs:161).
- Defaults where a value cannot be absent: MA `usize::try_from(max).unwrap_or(1)` with a comment saying it cannot fail (MA/src/handler.rs:247), MO `unwrap_or(usize::MAX)` (MO/src/queue.rs:336), QE `unwrap_or(0)` on a required Seconds (QE/src/broker.rs:90). The top crates write `expect("...is at most 10")` (BO/src/queue.rs:378).
- Silent `if let Some(msg) = messages.get_mut(key)` right after a lookup that just proved the key exists (LE/src/queue.rs:419, SU/src/queue.rs:441, MA/src/queue.rs:404). This hides bugs instead of asserting.

## Patterns seen in most crates
- Errors are a closed `ErrorCode` enum plus a free-text `message: String`, built through small constructors (`missing`, `invalid_value`). This is acceptable typing. Only BO carries typed reasons per variant (BO/src/error.rs:7-49).
- I/O errors always go up with `?` to `main`, which prints them and exits with failure. No crate drops a write or flush result. Serialization failures are either `?`-mapped to `io::Error` or an `expect` naming why serialization cannot fail.
- Integer parameters: either `arbitrary_precision` with `as_i64`, or `as_i64/as_u64` without it, then a range check. Fractions and exponents are rejected everywhere. MA and VA saturate oversized integers on purpose, with a doc comment.
- Clock and counter arithmetic is mostly bare `+` or `saturating_add` on u64. The spec leaves behavior past 4e9 unspecified, so I did not count this against anyone.
