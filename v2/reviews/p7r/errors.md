# p7r / errors

## Method

Every crate is a small edit of `BASE` (about 2.4k lines each, tests untouched), so I scored mainly
what each attempt changed. The same steps for all 22:

1. `diff -ruN BASE/src <CODE>/src`, read as the added lines only.
2. A grep of the non-test source (everything above the first `#[cfg(test)]` in each file) for
   `.expect(`, `.unwrap()`, `as u*/i*` casts, `+=`, `checked_/saturating_/try_from`, `.ok()`,
   `let _`, `if let Some`, `unwrap_or` and `.remove(`. I counted the hits and read each one in
   context.
3. A check of the four error weaknesses already in `BASE`, to see which ones each crate kept:
   - **R**: 11 `params.string(..)?.expect("required")` calls that depend on an earlier
     `require(...)` call (BASE/src/broker/message_commands.rs:57).
   - **S**: a silent `if let Some(message) = ...message_mut(key)` in `ChangeMessageVisibility`,
     which turns an impossible lookup failure into success (BASE/src/broker/message_commands.rs:196).
   - **D**: dead-letter moves are dropped silently when the target queue is missing
     (BASE/src/broker/message_commands.rs:168).
   - **A**: `max_messages as usize` (BASE/src/broker/message_commands.rs:153). The value is limited
     to 1..=10, so the cast is safe, but it is unchecked.

What stays the same everywhere: one typed `Error { code: ErrorCode, message }` with a closed
`ErrorCode` enum, integer parsing through `u64::try_from` with a range check, and no `unwrap`
in non-test code. Time arithmetic (`now + timeout`) is bounded by the spec's 4e9 ceiling in every
crate, so it does not separate them.

## Scores

| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| VI | 4 | 1 | VI/src/broker/message_commands.rs:188 | VI/src/broker/message_commands.rs:167 | Fixes R, S and D (the dead-letter target becomes an `expect` that names its invariant); keeps only the bounded `as usize` |
| GU | 4 | 2 | GU/src/broker/message_commands.rs:190 | GU/src/broker/message_commands.rs:161 | `required_*` getters; in-flight lookups return `ReceiptHandleExpired` instead of being skipped; no cast; D kept |
| JA | 4 | 2 | JA/src/request.rs:161 | JA/src/broker/message_commands.rs:156 | `require` returns the values it checked, so no presence `expect` is left; each remaining `expect` names its invariant; D kept |
| JO | 4 | 2 | JO/src/queue.rs:194 | JO/src/broker/message_commands.rs:148 | `required_*` getters; S becomes an `expect` that names its invariant; no cast; D kept |
| LI | 4 | 2 | LI/src/broker/message_commands.rs:162 | LI/src/broker/message_commands.rs:147 | Fixes R, S and A; reports the in-flight result as a `bool` mapped to an error; D kept |
| VO | 4 | 2 | VO/src/broker/message_commands.rs:212 | VO/src/broker/message_commands.rs:184 | Fixes R, S and A; `expect` messages name their invariants; D kept |
| WE | 4 | 2 | WE/src/broker/message_commands.rs:206 | WE/src/broker/message_commands.rs:171 | Fixes R, S and A; `DeleteQueue` uses `remove(..).ok_or_else`; D kept |
| HO | 3 | 9 | HO/src/request.rs:168 | HO/src/broker/message_commands.rs:195 | `require` returns the values it checked and the cast is gone, but the silent S branch stays |
| WO | 3 | 9 | WO/src/broker/message_commands.rs:202 | WO/src/broker/message_commands.rs:45 | Fixes S with errors, but keeps 12 presence `expect`s (now with invariant messages) and `as usize` |
| ZO | 3 | 9 | ZO/src/request.rs:117 | ZO/src/broker/message_commands.rs:196 | Fixes R and A; S stays |
| CA | 3 | 12 | CA/src/request.rs:125 | CA/src/broker/message_commands.rs:197 | Fixes R only; S and A stay |
| DA | 3 | 12 | DA/src/request.rs:123 | DA/src/broker/message_commands.rs:214 | Fixes R only; S and A stay |
| RU | 3 | 12 | RU/src/request.rs:117 | RU/src/broker/message_commands.rs:202 | Fixes R only; S and A stay |
| VA | 3 | 12 | VA/src/request.rs:82 | VA/src/broker/message_commands.rs:198 | Fixes R only; S and A stay |
| ZE | 3 | 12 | ZE/src/request.rs:131 | ZE/src/broker/message_commands.rs:206 | Fixes R only; S and A stay |
| RO | 3 | 17 | RO/src/broker/message_commands.rs:90 | RO/src/broker/message_commands.rs:224 | Rewords the 12 presence `expect`s to name `Params::require` and drops the cast; the coupling and S stay |
| CE | 2 | 18 | CE/src/broker/message_commands.rs:132 | CE/src/broker/message_commands.rs:67 | Only change is a `try_from` whose `expect` names its invariant; R, S and D all stay |
| BASE | 2 | 19 | BASE/src/request.rs:100 | BASE/src/broker/message_commands.rs:196 | Typed errors and checked integer parsing, but R, S, D and A |
| HE | 2 | 19 | HE/src/request.rs:94 | HE/src/broker/message_commands.rs:228 | Error constructor helpers only; R, S, D and A unchanged |
| NE | 2 | 19 | NE/src/request.rs:94 | NE/src/broker/message_commands.rs:216 | Error constructor helpers only; R, S, D and A unchanged |
| SA | 2 | 19 | SA/src/request.rs:102 | SA/src/broker/message_commands.rs:226 | Error-side changes are only moved checks; R, S, D and A unchanged |
| XU | 2 | 19 | XU/src/request.rs:137 | XU/src/broker/message_commands.rs:192 | Only the command enum and a helper; R, S, D and A unchanged |

## What separates the top from the bottom

- **Presence becomes part of the type.** The top crates make "required" a getter that returns
  `Result<T>` (GU/src/request.rs:120, VO/src/request.rs:122), or make `require` hand back the
  values it checked (JA/src/request.rs:161, HO/src/request.rs:168). That removes 11 or 12 `expect`
  calls whose safety depends on an earlier `require` line. The bottom tier keeps
  `expect("required")` (XU/src/broker/message_commands.rs:53). RO and WO keep the coupling and only
  reword the message (RO/src/broker/message_commands.rs:90).
- **The silent ChangeMessageVisibility branch.** The top tier replaces
  `if let Some(message) = ...message_mut(key)` with an error (GU/src/broker/message_commands.rs:190,
  WE/src/broker/message_commands.rs:206) or with an `expect` that states why the key exists
  (JO/src/queue.rs:194, VO/src/broker/message_commands.rs:212). Everyone else keeps a branch for a
  state that cannot occur, which reports success if it ever does occur
  (ZE/src/broker/message_commands.rs:206).
- **Dead-letter moves.** Only VI turns the silently skipped missing dead-letter target into an
  `expect` that names its invariant (VI/src/broker/message_commands.rs:188). It also replaces
  `dead_letters.extend(queue.remove(key))`, which dropped an `Option`, with an `expect`
  (VI/src/broker/message_commands.rs:176). Every other crate still loses those messages silently.
- **The cast.** Most top crates pass `max_messages` as `u64` all the way down
  (HO/src/broker/message_commands.rs:157), and CE uses `usize::try_from` with an `expect`
  (CE/src/broker/message_commands.rs:132). The rest keep `as usize`
  (VA/src/broker/message_commands.rs:157). It is safe because the value is at most 10, but it is
  the only unchecked conversion left.
- **New `expect`s are documented.** Every `expect` added by an attempt names its invariant
  ("a FIFO queue numbers its messages", WE/src/broker/message_commands.rs:123). None introduces a
  bare `unwrap` or a panic that input can trigger.

## Patterns seen in most crates

- All 22 keep the typed `Error`/`ErrorCode` pair and return `Result` from every handler. No
  `Result` is discarded, there are no `let _ =` and no `unwrap` in non-test code, and input
  integers go through `u64::try_from` with a range check. The floor is therefore high: no crate
  scores below 2.
- Most crates move the FIFO send checks into a helper (`fifo_ids`, `FifoSend`, `SendRules`) and the
  error behaviour stays the same. That move is a simplicity change, not an error-handling change,
  so it earned nothing here.
- 21 of 22 keep the silent dead-letter skip (D), and 14 of 22 keep the silent visibility branch
  (S). Both are inherited from `BASE` and look like code from a refactor that nobody re-read for
  states that cannot happen.
- `deliver`'s `expect("planned message exists")` (BASE/src/queue.rs:234) survives everywhere. It is
  a reasonable invariant, if terse.
