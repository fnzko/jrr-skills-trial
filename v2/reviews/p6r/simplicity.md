# p6r / simplicity

## Method

I read all of BASE `src/` (about 1,260 lines). Tests are byte-identical to BASE in every attempt,
so only `src/` was compared. For each attempt I produced `diff -ru BASE/src <CODE>/src`, read every
hunk, and read every file the attempt added (`ids.rs`, `time.rs`, `limits.rs`). To keep the method
the same everywhere, I ran the same greps across all 19 crates:
- `expect("required")`: the require-then-expect pattern, where one decision is made in two places;
- `content_based_deduplication == Some(true)`: the CBD rule copied between create and set;
- `message_mut` / `queue_mut(`: the double queue lookup in DeleteMessage and ChangeMessageVisibility;
- `#[allow`, `dbg!`, `println!`, `todo!`, `unreachable!`, `macro_rules`.

Five BASE lapses served as the reference points:
1. The CBD check is written twice (BASE/src/broker/queue_commands.rs:42 and :95).
2. The dead-letter pairing is decided both in `dead_letter()` and in `apply` (BASE/src/attributes.rs:84).
3. `require` is followed by eleven `expect("required")` calls.
4. DeleteMessage and ChangeMessageVisibility look the queue up twice, and keep an `if let` on a
   `None` that cannot happen (BASE/src/broker/message_commands.rs:196).
5. The error helpers are duplicated across modules.

Each attempt was scored on which of these it removed, and on what new structure it added that has
only one user.

## Scores

| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| QO | 3 | 1 | QO/src/attributes.rs:128 | QO/src/broker/message_commands.rs:193 | Fixes lapses 1-4 (CBD once, dead letter decided once, no expects, single lookup). The new free `in_flight_message` brings a now/issued/lookup preamble that is copied in two handlers (:179, :193). |
| KE | 3 | 2 | KE/src/broker/queue_commands.rs:26 | KE/src/attributes.rs:86 | Fixes lapses 1-3 with small helpers. Leaves the double lookup. `apply` ignores fields (`dead_letter_queue: _`) and then tests `is_some()`. |
| FO | 3 | 3 | FO/src/broker/message_commands.rs:197 | FO/src/broker/queue_commands.rs:95 | Fixes lapses 2-4 (returns `(&mut Queue, key)` and drops the single-use `has`). The CBD check is still copied. |
| PO | 2 | 4 | PO/src/attributes.rs:125 | PO/src/broker/queue_commands.rs:21 | A minimal, clean change: CBD written once and one shared missing-parameter error. All other BASE lapses remain. |
| FI | 2 | 5 | FI/src/attributes.rs:126 | FI/src/broker/queue_commands.rs:21 | Same as PO plus a `Command` enum. The expects and the double lookup remain. |
| BE | 2 | 6 | BE/src/broker/queue_commands.rs:19 | BE/src/limits.rs:8 | CBD written once. The new `limits` module claims every range has several users, but RETENTION, MAX_RECEIVE_COUNT and TICK have one each. |
| MI | 2 | 7 | MI/src/broker/queue_commands.rs:19 | MI/src/ids.rs:6 | CBD written once and impossible `if let`s become expects. It adds a `macro_rules` for three counters and a crate-wide pub-to-pub(crate) churn. Expects remain. |
| TE | 2 | 8 | TE/src/broker/message_commands.rs:155 | TE/src/broker/queue_commands.rs:42 | Replaces an impossible `and_then` branch with a named invariant. Otherwise a types change: CBD, expects and the double lookup all remain. |
| DE | 2 | 9 | DE/src/ids.rs:22 | DE/src/broker/message_commands.rs:21 | Takes the FIFO rules out into a five-parameter `fifo_ids` with one caller. CBD is still copied. |
| PA | 2 | 10 | PA/src/queue.rs:191 | PA/src/request.rs:73 | `Request::command()` parses the name on demand instead of once at the boundary. The double lookup (:161, :172) and the CBD copy remain. |
| BASE | 2 | 11 | BASE/src/broker.rs:100 | BASE/src/broker/queue_commands.rs:95 | Small and direct, but has the five lapses listed under Method. |
| NE | 2 | 12 | NE/src/broker/message_commands.rs:198 | NE/src/broker/message_commands.rs:153 | `queue_and_handle` removes the double lookup. A new single-use `move_to_dead_letter_queue` looks up again the dead-letter name that the caller already had. Message is built with `fifo: None` and then overwritten (:96). `take_next` is copied per id type. |
| ZE | 2 | 13 | ZE/src/error.rs:55 | ZE/src/broker/message_commands.rs:57 | Groups the error constructors. Adds a single-use `FifoSend` enum and a six-argument helper. CBD is still copied. |
| XE | 2 | 14 | XE/src/error.rs:58 | XE/src/broker/message_commands.rs:62 | Nearly the same as ZE: a single-use `FifoSend` and `check_fifo` with six parameters. CBD is copied and the expects remain. |
| PE | 2 | 15 | PE/src/attributes.rs:136 | PE/src/broker/message_commands.rs:58 | CBD written once, but it adds both `FifoIds` and `FifoSend` for one caller. |
| PI | 1 | 16 | PI/src/broker/message_commands.rs:202 | PI/src/attributes.rs:87 | The new `DeadLetterParams::Partial` needs an `unreachable!` branch in `apply`. Eleven three-line expects. Single-use `attributes()` / `forget_dead_letter_queue` / `apply_attributes` wrappers. |
| KU | 1 | 17 | KU/src/broker/queue_commands.rs:19 | KU/src/attributes.rs:70 | Makes the fields private behind about ten trivial getters. `Queue::clear_dead_letter_to` only forwards to `Attributes` (queue.rs:117). Eleven three-line expects. |
| CE | 1 | 18 | CE/src/request.rs:168 | CE/src/request.rs:169 | The array-returning `require` does remove the expects, but at a cost: a const generic, a `static NULL` sentinel, and typed getters duplicated on `Param` and `Params`. It also adds a `Delivery` enum with accessor methods, a `NewMessage` struct, and `Arc<str>` to avoid one clone. CBD is still copied. |
| JI | 1 | 19 | JI/src/broker/message_commands.rs:193 | JI/src/broker/queue_commands.rs:95 | The largest rewrite: `time` module, `QueueName` with Borrow/Display, a `QueueKind` enum, `DeadLetterUpdate`, `Message::new` and the CE-style `require`. Even with `QueueKind`, the CBD check is still written twice. |

## What separates the top from the bottom

- **The top crates fix BASE's duplicated decisions with the smallest tool available.**
  - QO, KE and FO move the dead-letter pairing to one place by passing `dead_letter()`'s result into
    `apply` (QO/src/attributes.rs:81).
  - QO and KE write the CBD rule once (KE/src/broker/queue_commands.rs:26).
  - None of them adds a type.
- **The double lookup separates FO and QO from the rest of the small changes.**
  - FO returns the queue together with the key (FO/src/broker/message_commands.rs:197).
  - PO, FI, BE, DE, TE and PA leave the second `queue_mut` and the `if let` on an impossible `None`.
- **Four attempts add a FIFO-send enum and helper with a single caller.**
  - PE (message_commands.rs:58), ZE (:57), XE (:62) and DE (:21) take the FIFO block out into a
    five- or six-parameter function, plus an enum to return through.
  - The handler gets no shorter, and readers now have one more indirection.
- **The bottom crates add structure in bulk.**
  - KU adds about ten getters on now-private fields, plus a forwarding method (KU/src/queue.rs:117).
  - CE adds a sentinel-based generic `require` (CE/src/request.rs:169).
  - PI adds an enum variant that exists only to be rejected, and an `unreachable!` to handle it
    (PI/src/attributes.rs:87).
  - JI adds new `time` and naming types.
  - All of them still keep at least one of BASE's copied rules (JI/src/broker/queue_commands.rs:42/:95).
- **The verbose expects show up in the bottom crates.** PI and KU change one-line
  `expect("required")` into eleven three-line expects each (PI/src/broker/message_commands.rs:39).
  This adds lines without removing the decision that is made twice.

## Patterns seen in most crates

- Every attempt removes the local `missing()` helper in favour of one shared missing-parameter
  error constructor.
- No crate has `#[allow]`, debug output or dead code.
- Ten of the 18 attempts swap the string dispatch for a `Command` enum. That change is neutral
  for simplicity: one more type, and the string match is the same size.
- Ten attempts still have the CBD check in two places, as BASE does (the grep finds two hits in
  CE, DE, FO, JI, NE, PA, PI, TE, XE and ZE).
- Every crate that removes `expect("required")` keeps the separate `require` call. The presence
  check therefore still runs twice. Only CE and JI fold the two into one call, and both pay for it
  with extra machinery.
- Module layout stays proportionate in most crates. The exceptions are BE (`limits.rs` holding five
  constants) and JI (`time.rs`, `ids.rs` and a much larger `naming.rs` for a 1,300-line crate).
