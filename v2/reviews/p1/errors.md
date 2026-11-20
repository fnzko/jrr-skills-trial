# p1 / errors

## Method

No `BASE` directory exists, so every crate was judged on its whole `src/`. The same steps were applied to all 15 crates, `src/` only (`tests/` and `#[cfg(test)]` modules were skipped):

1. One grep over `src/` for `.unwrap()`, `.expect(`, `panic!`, `unreachable!`, `unwrap_or*`, `.ok()`, `let _ =`, `as u*/i*` casts, `map_err(|_|`, `Err(_)`, `if let Ok`, `serde_json::to_*` and slicing/indexing. Every hit outside test code was classified.
2. Read in full: `main.rs` (or `bin/miniq.rs`) and `error.rs`.
3. Read in full: the integer-parameter parser, the receipt-handle parser and the `ReceiveMessage` handler, including the dead-letter hand-off. That hand-off is where every crate meets the "dead-letter queue must exist" invariant.
4. Read the surrounding code of every `expect`, cast or silent fallback found in step 1.

Spec defaults (`unwrap_or(30)`, a missing `Prefix` read as "") were not counted as failures turned into defaults.

## Scores

| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| FU | 4 | 1 | FU/src/scalar.rs:24 | FU/src/command.rs:265 | range-typed `Bounded` via checked `TryFrom`; every `expect` names its invariant; only nit: `.ok()` drops the `OutOfRange` detail |
| FI | 4 | 1 | FI/src/queue.rs:24 | FI/src/broker.rs:82 | a named `INDEXED` invariant constant, `usize::try_from` + named expect; the clock saturates silently instead of stating that it cannot overflow |
| HE | 4 | 1 | HE/src/values.rs:48 | HE/src/broker.rs:361 | typed `Ranged` values, saturating `Instant`, rendering returns `Result`; two raw `[index]` lookups on internal indices carry no stated invariant |
| WU | 4 | 1 | WU/src/service.rs:237 | WU/src/params.rs:69 | no casts, named expects, the dead-letter invariant is asserted; the integer parse flattens its error with `.ok()` (the message is still accurate) |
| ZE | 4 | 1 | ZE/src/receipts.rs:41 | ZE/src/request.rs:132 | handle lookup returns a typed `Error`, checked index math; one `try_from(..).expect` that a bounded type would remove |
| QO | 3 | 6 | QO/src/types.rs:146 | QO/src/broker.rs:312 | NonZero handles and range types throughout; an impossible dead-letter lookup is answered with `?`, which can return a client error after state was already changed |
| VU | 3 | 6 | VU/src/queue.rs:102 | VU/src/queue.rs:155 | checked handle-index conversion and named expects; `u64::MAX` sentinel, and saturating at :146 but unchecked at :149 |
| BI | 3 | 6 | BI/src/service.rs:326 | BI/src/service.rs:316 | invariants named at every expect; `max_messages as usize` and a `u64::MAX` sentinel (queue.rs:216) |
| CE | 3 | 6 | CE/src/request.rs:112 | CE/src/broker.rs:431 | require-then-expect, but the message names the check; `unreachable!` branch to unwrap type-erased `json!` values; `as usize` at :305 |
| TA | 3 | 6 | TA/src/queue.rs:35 | TA/src/broker.rs:309 | named `INDEXED` invariant; `panic!` branch for a non-object `json!`, `len() as u64` cast (queue.rs:325) |
| TO | 3 | 6 | TO/src/receipt.rs:55 | TO/src/queue.rs:437 | handle resolution returns a typed error; `change_visibility` silently does nothing when the message is not in flight, `delete` silently ignores a missing message (:428) |
| LI | 2 | 12 | LI/src/params.rs:53 | LI/src/broker.rs:236 | careful integer parse, but a missing dead-letter queue silently discards the moved messages; `try_from(..).unwrap_or(usize::MAX)` (:232) |
| MI | 2 | 12 | MI/src/command.rs:247 | MI/src/broker.rs:272 | `let Ok(target) = lookup(..)` swallows an error and loses dead-lettered messages; `first_received_at.unwrap_or(now)` (queue.rs:363) hides an invariant; `n as usize` |
| PO | 2 | 12 | PO/src/request.rs:100 | PO/src/broker/message_commands.rs:168 | silent drop of dead letters, `if let Some(message)` no-op (:196); `require(..)` then `.expect("required")` repeated about 10 times |
| BO | 1 | 15 | BO/src/params.rs:69 | BO/src/broker.rs:90 | required `Seconds` defaulted with `unwrap_or(0)`; handle defaulted to 0 (message_commands.rs:183); 6 `as` casts; silent dead-letter drop (:104); about 12 `expect("required")` |

## What separates the top from the bottom

- **Dead-letter hand-off.** Every crate has to send moved messages to a queue that must exist.
  - The top crates assert it with a named invariant: FU/src/service.rs:329, WU/src/service.rs:237, ZE/src/broker.rs:127, VU/src/broker.rs:245.
  - The bottom four skip it silently and lose the messages: BO/src/message_commands.rs:104, PO/src/broker/message_commands.rs:168, LI/src/broker.rs:236, MI/src/broker.rs:272.
- **Required parameters.** BO and PO check presence in one call and re-extract with `.expect("required")` in another (BO/src/message_commands.rs:19-26, PO/src/broker/queue_commands.rs:21). BO even reads the required `Seconds` with `unwrap_or(0)` (BO/src/broker.rs:90). FU, HE, QO and ZE parse each parameter once into a typed value (`required_bounded`, `required_string`), so no follow-up expect is needed.
- **Numeric conversion.** All 15 crates read input integers with checked `as_i64`/`as_u64` + `try_from`. After that:
  - BO casts freely (`n as u32`, `seconds as u64`, `v as u32`, `number as usize - 1`: BO/src/queue.rs:95, BO/src/message_commands.rs:97,148).
  - FU, HE and QO carry range types (`Bounded`, `Ranged`, `MaxMessages`), so no cast is needed later.
- **Silent no-ops on invariant breaks.** `if let Some(message) = ..` that does nothing when the "impossible" case happens: BO/src/message_commands.rs:137, PO/src/broker/message_commands.rs:196, TO/src/queue.rs:437. FI, TA and WU state the same invariant with a named expect (WU/src/queue.rs:379).
- **Impossible branches.** CE and TA keep an `unreachable!`/`panic!` arm because they build responses as `json!` and unwrap them back (CE/src/broker.rs:431, TA/src/broker.rs:309). QO turns an impossible lookup into a client-visible `QueueDoesNotExist` (QO/src/broker.rs:312).

## Patterns seen in most crates

- **Error type.** All 15 use the same shape: a closed `ErrorCode` enum plus a `message: String`. None uses `String` as the error type. Only CE has a typed internal error (`HandleExpired`); everywhere else, internal failures are `Option`s collapsed at the boundary, which is acceptable here because each has a single cause.
- **I/O errors.** No crate drops a `Result`: there is no `let _ =` anywhere in `src/`, and every `main` propagates I/O errors to a non-zero exit.
- **Receipt handles.** Parsing is uniformly careful (no leading zero, digits only, checked parse, bounds check before indexing), so no input string can cause a panic in any crate.
- **Expect messages.** Most remaining `expect`s name a real invariant ("DeleteQueue clears every reference..."). The weak ones are BO's and PO's bare `"required"`.
