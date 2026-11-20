# p4 / errors

## Method
I ran the same checks on all 12 crates (no BASE directory exists). Production code means each `src/*.rs` file up to its `#[cfg(test)]` line.
- One grep over `src/` for `unwrap()`, `expect(`, `panic!`, `unreachable!`, `todo!`, `unimplemented!`. I then read every hit in production code to see whether its message names an invariant.
- One grep for silent-default and dropped-result patterns: `unwrap_or*`, `.ok()`, `let _ =`, `filter_map`, `map_err(|_|`, `Err(_)`, `as <int>` casts, `saturating_*`/`wrapping_*`, `Result<_, String>`, and `None => {}`/`else { return }`.
- One grep for arithmetic operators (`+ - *`, `+=`) in production code, and one for `checked_*` and `as_u64`/`as_i64`. I also checked `Cargo.toml` for serde_json's `arbitrary_precision`.
- I read every `error.rs` and `main.rs`, the stdin/stdout loop (I/O and serialization result handling), the integer-parameter parser, the receipt-handle parser, and the context around each suspicious hit.
- I did not read the tests themselves. I left aside `unwrap_or` that applies a spec default (an absent optional parameter), since that is not a failure turned into a default.

## Scores
| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| MU | 4 | 1 | MU/src/request.rs:318 | MU/src/types.rs:42 | Typed thiserror `Error` enum, the integer parser reads the source text and accepts `-0`, every production `expect` names an invariant. The only lapse: identifier parsers return `Result<_, String>` (the rejected input). |
| WE | 4 | 1 | WE/src/error.rs:26 | WE/src/queue.rs:240 | Typed thiserror enum mapped to codes, `arbitrary_precision` plus checked `u32::try_from` (request.rs:278). One `unreachable!` arm for a dead-letter type mismatch that the types allow. |
| PA | 3 | 3 | PA/src/types.rs:46 | PA/src/queue.rs:719 | Id, clock and arrival counters all use `checked_add` with invariant-naming `expect`. But there is an `unreachable!` arm for a FIFO/standard mismatch, and `as_i64` rejects the valid `-0` (request.rs:330). |
| QI | 3 | 3 | QI/src/time.rs:25 | QI/src/queue.rs:290 | Checked clock and counter arithmetic. `unreachable!` covers a group/FIFO pairing the types do not rule out, `Option` fields are `expect`ed at reply.rs:157/163, and `-0` is rejected (params.rs:37). |
| KE | 3 | 3 | KE/src/request.rs:212 | KE/src/dedup.rs:57 | Exact integer parsing over the source text (accepts `-0`). Several `expect` messages only restate the call ("front exists", "removed message exists", queue.rs:496), and there is an `unreachable!` on the result serialization shape (service.rs:312). |
| ZU | 3 | 3 | ZU/src/request.rs:284 | ZU/src/queue.rs:320 | Source-text integer check plus checked `i64` -> `u32` conversion. A `panic!` arm for a dead-letter type mismatch the types allow. |
| TA | 3 | 3 | TA/src/queue.rs:163 | TA/src/command.rs:275 | No unreachable arms. `checked_sub` for retention, checked `u32::try_from`, invariant-naming `expect`s. `as_u64` without `arbitrary_precision` rejects `-0`. |
| XI | 3 | 3 | XI/src/request.rs:245 | XI/src/request.rs:337 | Clean expects and a checked `usize` conversion. `as_u64` rejects `-0`. Clock math is plain `+` (bounded by the spec). |
| FU | 3 | 3 | FU/src/queue.rs:330 | FU/src/request.rs:337 | Typed `HandleExpired` error from the store, all I/O propagated. Rejects `-0`, and `expect("front exists")` at queue.rs:460 names no invariant. |
| NI | 2 | 10 | NI/src/session.rs:35 | NI/src/params.rs:28 | Serialization and I/O errors propagated. A doc comment admits the `-0` bug and leaves it in. Also an `as u64` cast in the receive loop (broker.rs:348) and a clock that silently saturates (broker.rs:432). |
| QE | 2 | 10 | QE/src/broker.rs:377 | QE/src/broker.rs:287 | Receipt index uses `checked_sub` + `try_from`. But messages planned for dead-lettering that go missing are dropped silently with `filter_map`, while the same invariant is `expect`ed four lines later. Rejects `-0` (wire.rs:17); unchecked `now +` arithmetic (queue.rs:110). |
| SO | 2 | 10 | SO/src/broker.rs:455 | SO/src/broker.rs:353 | Checked handle-to-index conversion. But it casts with `as` (`max_messages as usize` at broker.rs:344, `receipts.len() as u64 + 1` at 353), uses unchecked `+=`/`+` on state, rejects `-0` (params.rs:93), and its expects are vague ("planned messages exist"). |

## What separates the top from the bottom
- **Strict integer parsing.** MU, WE, KE and ZU turn on `arbitrary_precision` and check the number's source text (MU/src/request.rs:318, ZU/src/request.rs:284), so `-0` is accepted and `30.0`/`3e1` are rejected. The other eight use `as_u64`/`as_i64` on a plain `Value`, which rejects the valid integer `-0` with `InvalidParameterValue`. NI documents this (NI/src/params.rs:28) and ships it anyway.
- **Typed error values.** MU and WE give each failure reason its own variant of a thiserror enum (MU/src/error.rs:27, WE/src/error.rs:26), mapped to the wire code in one place. Everyone else uses a `{code: ErrorCode, message: String}` struct: the code is typed, but the reason is free text.
- **Counters and the clock.** PA and QI make every counter and clock step checked, with an `expect` that says why overflow cannot happen (PA/src/types.rs:63, QI/src/time.rs:36). SO uses `as` casts and unchecked `+=` (SO/src/broker.rs:344, 353), and QE and XI use plain `now + x`. These are bounded by the spec in practice, but nothing in the code says so.
- **Branches for impossible states.** PA, QI, WE and ZU each keep an `unreachable!`/`panic!` arm for a FIFO/standard pairing their types allow (PA/src/queue.rs:719, QI/src/queue.rs:290, WE/src/queue.rs:240, ZU/src/queue.rs:320). TA, XI, MU and FU have none.
- **Consistency within a crate.** QE handles the same "planned message is still stored" invariant in two ways, dropping it silently at QE/src/broker.rs:287 and `expect`ing it at broker.rs:312. KE and FU have `expect` messages that only restate the call (KE/src/dedup.rs:57, FU/src/queue.rs:460).

## Patterns seen in most crates
- No production `unwrap()` anywhere, no `let _ =` dropping a `Result`, and every stdin/stdout loop propagates read, write, flush and serialization errors with `?` and exits non-zero on I/O failure.
- `ErrorCode` is a closed enum in all 12 crates. None compares error codes as strings.
- Nearly every production `expect` sits on a cross-structure invariant (dead-letter target exists after `DeleteQueue` cleanup, index entries name stored messages), and most messages say why it holds.
- Invalid UTF-8 and bad JSON become `InvalidRequest` responses rather than crashes in every crate. No input-reachable panic was found in any of them.
- No crate scored below 2: the problems found are edge cases and style lapses, not failures turned into defaults or swallowed errors.
