# p6g / errors

## Method
No BASE directory exists, so every crate was judged in full. The same steps ran on every crate:
1. A script scanned all non-test code under `src/` (it stopped at the first `#[cfg(test)]` in each
   file and skipped `tests.rs`) for `.unwrap()`, `.expect(`, `panic!`, `unreachable!/todo!`, `as`
   integer casts, `unwrap_or*`, `let _ =`, `.ok()`, `checked_/saturating_`, and indexing. I read
   every hit except the routine `unwrap_or` for attribute defaults.
2. Read `main.rs` (how I/O errors end up) and `error.rs` (the shape of the error type) in each.
3. Read the integer parameter parser (the `as_i64`/`as_u64` site) in each, to check type and range
   handling and the conversion into the target type.
4. Grepped every `if let Some(...) =`, `if let Ok` and `let ... else` on a lookup, and read the
   dead-letter move and the visibility change in each, to find failures silently skipped.
5. Searched for `Result<_, String>`, `type Err =` and `Box<dyn Error>`. There were no hits apart from
   DE's typed `FromStr` errors.
`cargo` was not run, and tests were not judged.

## Scores
| code | score | rank | best evidence | worst evidence | reason |
|---|---|---|---|---|---|
| CA | 4 | 1 | CA/src/error.rs:52 | CA/src/types.rs:72 | One enum variant per protocol code, with typed sub-reasons (`RequestFault`, `Limit`). The only lapse is a `saturating_sub` that hides a broken "earlier <= now" precondition. |
| CE | 4 | 1 | CE/src/scalar.rs:26 | CE/src/command.rs:265 | Bounded `TryFrom<i64>` with a typed `OutOfRange`. That error is then dropped with `.ok()` in favour of a generic message. |
| DE | 4 | 1 | DE/src/domain.rs:107 | DE/src/broker.rs:82 | Typed `FromStr` errors for names and ids, and expects that share one named invariant (`INDEXED`). The clock silently saturates. |
| NE | 4 | 1 | NE/src/request.rs:306 | NE/src/time.rs:30 | `Ranged::try_from` passes its error into the message. Expects name the invariant they rely on. `since` returns 0 for a future instant instead of failing. |
| QA | 4 | 1 | QA/src/request.rs:131 | QA/src/error.rs:26 | Checked conversions, every expect named, no silent skips. It shares the common enum code plus free `String` message. |
| RU | 4 | 1 | RU/src/request.rs:235 | RU/src/error.rs:24 | Same as QA: checked `u32::try_from`, expects named (`request.rs:196`), nothing swallowed. |
| TA | 4 | 1 | TA/src/request.rs:205 | TA/src/error.rs:26 | `try_from(..).expect("MaxReceiveCount is at most 1000")` throughout. No casts, no silent skips. |
| JU | 3 | 8 | JU/src/queue.rs:20 | JU/src/broker.rs:312 | Shared named invariant constant and saturating counters. But after `receive` has already mutated state, a `?` branch exists for a dead-letter queue that cannot be missing. |
| KO | 3 | 8 | KO/src/request.rs:186 | KO/src/service.rs:316 | Checked parsing, but a `max_messages as usize` cast and a `u64::MAX` "no messages" sentinel (`queue.rs:216`). |
| LA | 3 | 8 | LA/src/service.rs:265 | LA/src/request.rs:218 | Expects state invariants well. One `as usize` cast on input. |
| NI | 3 | 8 | NI/src/broker.rs:304 | NI/src/queue.rs:441 | Dead-letter lookup is an expect that names its invariant. The visibility change silently does nothing if the message is missing. |
| NO | 3 | 8 | NO/src/request.rs:61 | NO/src/queue.rs:319 | Named expects. `unreachable!` covers a phase the types could have ruled out, and `params.rs:142` expects that two validators agree. |
| TU | 3 | 8 | TU/src/request.rs:285 | TU/src/request.rs:189 | `Bounded::new` is checked. Required parameters are checked by a separate `require` list and then `expect`ed, so one missed name would panic on input. |
| VE | 3 | 8 | VE/src/request.rs:202 | VE/src/queue.rs:437 | Checked parsing and named expects. One `max_messages as usize` cast. |
| HU | 2 | 15 | HU/src/params.rs:49 | HU/src/broker.rs:431 | `unreachable!` checks a value shape at runtime. Map indexing `members["Attributes"]` (`request.rs:81`) is guarded only by an earlier `require`. Visibility change silently skips (`queue.rs:337`), plus an `as usize` (`broker.rs:305`). |
| SO | 2 | 15 | SO/src/command.rs:251 | SO/src/broker.rs:272 | `if let Ok(target) = lookup(..)` drops a `Result` and the dead letters with it. `first_received_at.unwrap_or(now)` defaults an impossible state (`queue.rs:363`), there is a silent skip (`queue.rs:418`), and an `n as usize`. |
| JI | 1 | 17 | JI/src/message_commands.rs:177 | JI/src/broker.rs:90 | A required `Seconds` is defaulted with `unwrap_or(0)`. `latest_handle.unwrap_or_default()` would emit `r-0` (`message_commands.rs:183`). About 12 `.expect("required")`, 9 `as` casts on input, raw indexing (`message_commands.rs:148`), and silent dead-letter and visibility skips (`:104`, `:137`). |
| LI | 1 | 17 | LI/src/request.rs:101 | LI/src/broker/message_commands.rs:168 | Dead letters are silently dropped if the target is missing. About 10 `.expect("required")` that name no invariant, a silent visibility skip (`:196`), and `as usize` (`:153`). |

## What separates the top from the bottom
- **Where required-ness lives.** The top crates have a `required(name)?` that returns the value, so
  absence becomes `MissingParameter` in one place. JI, LI, HU and TU check a list with
  `require(&[...])` and then `expect` each value (JI/src/queue_commands.rs:82,
  LI/src/broker/message_commands.rs:57, TU/src/request.rs:189). That couples two call sites, and a
  mismatch panics on input.
- **Silent skips of impossible states versus named expects.** CA, DE, NE, NI, QA, RU, TA and VE
  `expect` the dead-letter target with a reason (DE/src/broker.rs:313). JI and LI skip it with
  `if let Some` (JI/src/message_commands.rs:104), and SO drops the `Result` (SO/src/broker.rs:272).
  Either way, messages would vanish without any signal.
- **Defaults for missing state.** JI emits `r-{latest_handle.unwrap_or_default()}` and
  `unwrap_or(0)` for a required parameter. SO uses `unwrap_or(now)` for an unset `FirstReceivedAt`.
  No top crate turns an absent value into a made-up one.
- **Conversions.** The top crates use `u32::try_from`/`usize::try_from(..).expect("at most 10")`
  (QA/src/broker.rs:315). JI casts range-checked `i64` with `as u32`/`as u64` in 9 places, and
  KO, LA, VE, HU, SO and LI each keep one `as usize` on `MaxMessages`.
- **Error typing.** CA alone models each protocol code as an enum variant with typed causes
  (CA/src/error.rs:7-73). Every other crate uses `ErrorCode` plus a free `String` message. The spec
  leaves the message text free, so this is not a defect, but it is less checkable.

## Patterns seen in most crates
- No production `.unwrap()` anywhere, no `let _ =`, and no `Result<_, String>`. I/O errors propagate
  out of `main` with a non-zero exit in all 18 crates.
- Integer parameters go through `as_i64`/`as_u64`, then a checked `try_from` and a range filter, so
  `30.0`, `3e1`, negatives and huge values all become `InvalidParameterValue` rather than panics.
- Response serialization is `expect`ed with a reason ("only strings, integers and booleans") in
  most crates, which is a justified invariant.
- Clock arithmetic is plain `+` or `saturating_add` on `u64`. The spec bounds the clock, so neither
  choice is a real risk.
