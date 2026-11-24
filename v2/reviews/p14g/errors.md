# p14g / errors

## Method
Same steps on all 12 crates, whole crate (no BASE), test modules excluded from findings:
- Read `src/main.rs` and `src/error.rs` in full.
- Grepped `src/` for `unwrap(`, `expect(`, `panic!`, `unreachable!`, `.ok()`, `let _ =`, `unwrap_or*`,
  `map_err(|_|`, `as u*/i*` casts, `Result<_, String>`, slice indexing, and `+=`/`-`/`checked_`/
  `saturating_`; built a non-test listing (lines before `#[cfg(test)]`) and re-ran the panic/cast/
  discard grep on it.
- Read each crate's integer-parameter parser and its dead-letter move in ReceiveMessage, the two
  places where input conversion and impossible-state handling concentrate.
- Checked `Cargo.toml` for `arbitrary_precision` (affects whether `30.0`/`3e1` are rejected).
No crate has an input-reachable panic I could find; I/O errors are propagated in every `main`.

## Scores
| code | score | rank | best evidence | worst evidence | reason |
|---|---|---|---|---|---|
| XI | 4 | 1 | XI/src/error.rs:51 | XI/src/error.rs:77 | thiserror enum, one variant per code with typed causes; every expect names its invariant; only nit is `code()` returning `&'static str` instead of an enum |
| DO | 4 | 2 | DO/src/error.rs:23 | DO/src/queue.rs:304 | richest error model (variants, `#[source]` on Utf8/JSON errors, typed Problem/Limit); checked `T::try_from`; one `unreachable!` arm for a Fifo/standard mismatch that the types could rule out |
| ZO | 3 | 3 | ZO/src/handles.rs:34 | ZO/src/error.rs:48 | checked `checked_sub` + `try_from` on handle numbers, named expects, saturating time; error detail is a free-form `&str` |
| SE | 3 | 3 | SE/src/store.rs:31 | SE/src/error.rs:47 | range-typed `Bounded` built via `try_from`, checked handle math, named expects; error messages are untyped strings behind the code enum |
| PE | 3 | 5 | PE/src/broker.rs:304 | PE/src/error.rs:37 | missing DLQ is an `expect` with a reason, not a silent skip; checked `u32::try_from`; `invalid_value` takes any message string |
| PA | 3 | 5 | PA/src/request.rs:66 | PA/src/response.rs:100 | integers go through i128 then `u64::try_from`, bounded newtypes; serialization failure handled by `expect` (named) rather than propagated |
| CU | 3 | 5 | CU/src/request.rs:144 | CU/src/request.rs:205 | `as_u64` plus inclusive range; then `u32::try_from(..).expect` and `usize::try_from(..).expect` because the parser returns the wrong width |
| CI | 3 | 8 | CI/src/params.rs:82 | CI/src/queue.rs:404 | checked `u32::try_from`, named expects; `usize::try_from(max).unwrap_or(usize::MAX)` turns a failed conversion into a default |
| TA | 3 | 8 | TA/src/queue.rs:373 | TA/src/request.rs:138 | typed `NotInFlight` error mapped once to the protocol code; `as usize` cast on a parsed value instead of a checked conversion |
| HO | 2 | 10 | HO/src/command.rs:250 | HO/src/broker.rs:272 | checked parse of integers, `saturating_add` on the clock; DLQ lookup `let Ok(..)` discards an `Err` and silently drops moved messages; `_ => None` hides a half-set dead-letter config (HO/src/queue.rs:73) |
| NO | 1 | 11 | NO/src/request.rs:101 | NO/src/broker/message_commands.rs:168 | `require` then `.expect("required")` re-checks presence at about 11 sites; missing DLQ silently loses messages; `as usize` on a parsed value (message_commands.rs:153) |
| HU | 1 | 12 | HU/src/params.rs:31 | HU/src/message_commands.rs:148 | same `.expect("required")` pattern at about 14 sites, `unwrap_or(0)` default after `require` (broker.rs:90), seven `as` casts, raw `handles[n as usize - 1]` indexing, silent DLQ drop (message_commands.rs:104) |

## What separates the top from the bottom
- Required parameters: XI, DO, CU, PE, SE, TA, ZO, PA, CI parse each command into a typed request,
  so a required value is never `Option` afterwards. HU and NO call `require(&[..])` and then
  re-fetch each value with `.expect("required")` (HU/src/message_commands.rs:19, NO/src/broker/message_commands.rs:57),
  and HU even falls back to a default after the check (HU/src/broker.rs:90).
- Impossible states: PE, CU, CI, DO, PA, SE, TA, XI, ZO `expect` the dead-letter queue with a
  sentence naming why it exists (XI/src/service.rs:260). HO, HU and NO skip the move silently when
  it is missing, which would lose messages instead of failing loudly (HO/src/broker.rs:272).
- Numeric conversion: the best use `try_from`/`checked_sub` end to end (ZO/src/handles.rs:34,
  SE/src/store.rs:31). HU casts with `as` after range checks (HU/src/queue.rs:95,
  HU/src/message_commands.rs:97) and indexes a Vec with a cast (HU/src/message_commands.rs:148).
- Error model: DO and XI have one variant per code with typed causes and `#[source]`
  (DO/src/error.rs:75); the rest use `ErrorCode` + free `String` message, typed only by code.
- Conversion fallbacks: CI quietly maps a failed `usize::try_from` to `usize::MAX`
  (CI/src/queue.rs:404) where CU and XI at least `expect` with a reason (XI/src/queue.rs:378).

## Patterns seen in most crates
- Every crate propagates stdin/stdout errors with `?` from `main` or a `run` function, and none
  drops a `Result` with `let _ =`.
- All 12 have a closed `ErrorCode` enum covering exactly the ten protocol codes; ten of them pair it
  with a `String` message instead of structured causes.
- No input-reachable `unwrap`; non-test `expect`s are nearly all phrased as invariants.
- Integers are read with `as_i64`/`as_u64` (or string reparse under `arbitrary_precision`) and
  range-checked before narrowing; the lapses are `as` casts after the check, not unchecked input.
- `now - accepted_at` subtractions (HO, HU, NO) rely on the monotonic clock rather than
  `checked_sub`; safe here, but unguarded.
