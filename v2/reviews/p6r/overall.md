# p6r / overall

## Method
- Read SPEC.md, then every BASE source file in full (lib, error, naming, attributes, request, queue, broker, broker/*).
- For each of the 18 attempts I produced `diff -ruN BASE/src <CODE>/src` and read the whole diff, plus every
  file the attempt added (`ids.rs`, `time.rs`, `limits.rs`). No attempt touched `tests/`, so the integration
  tests were the same for all and were not a differentiator. Inline `#[cfg(test)]` changes were read as part of the diff.
- I then ran the same targeted greps on every crate: `expect("required")`, `max_messages as usize`, the silent
  dead-letter `and_then(|q| self.queues.get_mut` fallback, and the new id/enum definitions. That way each crate got the same look.
- BASE was scored as a crate in its own right. Each attempt was judged mainly on what its diff does: whether it
  makes later changes safer or more readable, and how much new machinery a maintainer has to learn before trusting it.
- No crate was compiled or run (cargo was not allowed). Nothing outside `v2/blind/p6r/` and the spec was read.

## Scores
| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| MI | 4 | 1 | MI/src/broker/message_commands.rs:153 | MI/src/ids.rs:6 | Private-field newtypes, Command parsed at the boundary, the silent DLQ drop turned into a documented invariant, `as` cast removed; only blemishes are a small macro and `pub(crate)` churn |
| TE | 4 | 2 | TE/src/broker/message_commands.rs:155 | TE/src/broker/message_commands.rs:38 | Focused newtypes (with tests) and Command enum, and the DLQ invariant is made explicit; keeps BASE's require-then-`expect("required")` pair |
| DE | 3 | 3 | DE/src/ids.rs:18 | DE/src/broker/message_commands.rs:163 | Small, clean MessageId/ReceiptHandle/Arrival newtypes plus one extracted FIFO check; `pub u64` fields and the silent DLQ fallback stay |
| PI | 3 | 4 | PI/src/queue.rs:35 | PI/src/attributes.rs:87 | Encapsulated attributes, FifoPosition and invariant expects; but DeadLetterParams keeps a `Partial` state that ends in `unreachable!`, and the expect text is repeated many times |
| JI | 3 | 5 | JI/src/naming.rs:20 | JI/src/request.rs:94 | Strongest types (QueueName with Borrow, Instant/Seconds, QueueKind, typed `require`); also the largest surface to learn, the MissingParameter error is built in two places, and DLQ drop is still silent |
| KU | 3 | 6 | KU/src/broker/message_commands.rs:160 | KU/src/attributes.rs:70 | Thorough ids incl. SequenceNumber and DLQ invariant; a dozen trivial getters and three copies of `take_next` add ceremony |
| CE | 3 | 7 | CE/src/request.rs:168 | CE/src/attributes.rs:24 | Typed `require` array and Delivery enum remove impossible states; `Arc<str>` used only to avoid one clone, `static NULL` placeholder trick |
| FO | 3 | 8 | FO/src/broker/message_commands.rs:197 | FO/src/attributes.rs:76 | `in_flight_message` returns the queue, so there is no second lookup, and `required_*` helpers are added; `apply` now ignores fields its argument still carries |
| QO | 3 | 9 | QO/src/broker/message_commands.rs:207 | QO/src/attributes.rs:81 | Same shape as FO, plus a free resolver fn; same awkward `apply(update, dead_letter)` |
| PA | 3 | 10 | PA/src/queue.rs:191 | PA/src/request.rs:73 | Newtypes plus a Command enum, but the command is parsed lazily at dispatch, not at the boundary |
| KE | 3 | 11 | KE/src/request.rs:78 | KE/src/attributes.rs:77 | Modest helper consolidation; an `impl AttributeUpdate` placed in another module, and the `apply` split |
| FI | 3 | 12 | FI/src/request.rs:16 | FI/src/broker/message_commands.rs:53 | Command enum and one dedup helper; otherwise BASE unchanged |
| BE | 3 | 12 | BE/src/request.rs:16 | BE/src/broker.rs:82 | Command enum and a `limits` module of range constants; otherwise BASE unchanged |
| PO | 3 | 14 | PO/src/attributes.rs:125 | PO/src/broker/message_commands.rs:53 | Tiny, safe dedup of two helpers and a checked usize conversion |
| PE | 3 | 15 | PE/src/attributes.rs:136 | PE/src/broker/message_commands.rs:58 | Named range constants and a one-caller FifoSend enum/helper; no real gain in safety |
| ZE | 3 | 16 | ZE/src/error.rs:55 | ZE/src/broker/message_commands.rs:52 | Error shorthands are a plus; `FifoSend::New(None)` for standard queues blurs the name |
| BASE | 3 | 17 | BASE/src/queue.rs:194 | BASE/src/broker/message_commands.rs:168 | Already readable and well split; bare u64 ids, string dispatch, `expect("required")` after `require`, DLQ silently skipped |
| NE | 2 | 18 | NE/src/queue.rs:212 | NE/src/broker.rs:24 | Good in-flight encapsulation and FifoPlacement, but `std::time::Duration` used for virtual instants (the comment admits it) and a `fifo: None` placeholder overwritten by `push` |
| XE | 2 | 19 | XE/src/broker/message_commands.rs:62 | XE/src/queue.rs:137 | ZE-style refactor plus a layer of doc comments that restate the code |

## What separates the top from the bottom
- **Making an implicit invariant explicit.** BASE silently skips dead-letter moves when the target is missing
  (BASE/src/broker/message_commands.rs:168), which would lose messages without a trace. MI:153, TE:155, KU:160 and PI
  turn this into an `expect` that names why it holds. Most other attempts leave the silent fallback in place
  (DE:163, JI:156).
- **Ids that cannot be mixed up, at the right size.** MI, TE and DE add only the newtypes that pay for themselves
  (MessageId, ReceiptHandle, arrival index) and keep `parse`/`Display` beside them. JI and CE go further (Instant/Seconds,
  QueueKind, Delivery, typed `require`). That is safer, but it doubles what a newcomer has to learn and adds tricks like
  placeholder `static` Values (JI/src/request.rs:94, CE/src/request.rs:169).
- **Parsing at the boundary.** Command enums built in `Request::parse` (MI, TE, JI, PI, NE, KU, CE, FI, BE) beat PA's
  lazy `req.command()?` at dispatch (PA/src/request.rs:73), which keeps the raw String inside the request.
- **Removing the second lookup.** Returning `(&mut Queue, key)` from the handle resolver (FO:197, PI, KU, JI, QO:207)
  removes BASE's look-up-twice-then-`if let Some` pattern. The weaker variant splits `apply(update, dead_letter)` and leaves
  fields destructured as `_` (FO/QO/KE attributes.rs), so the same data travels two ways.
- **Ceremony without payoff.** KU's getter wall (KU/src/attributes.rs:70 onward), PE/ZE/XE's one-caller `FifoSend`
  enums, and XE's restating doc comments (XE/src/queue.rs:137) add reading cost and no safety.
- **Odd type choices.** NE models virtual clock instants as `Duration` (NE/src/broker.rs:24). PI and CE keep a
  `Partial` enum state just to report it later; PI ends that state in `unreachable!` (PI/src/attributes.rs:87).

## Patterns seen in most crates
- Every attempt is a behavior-preserving cleanup of a BASE that was already in decent shape, so the spread is narrow
  (2 to 4). None of the crates is hard to inherit, and none touched the integration tests.
- Nearly all of them pull the `missing` / `MissingParameter` constructor into one helper and fold the duplicated
  `ContentBasedDeduplication` check into one function.
- About half still pair `params.require(..)` with `params.string(..)?.expect("required")`, so the same decision is still made twice (FI, BE, PO, PE, ZE,
  XE, TE, DE, MI). Only CE and JI make that structurally impossible, with a typed `require` that returns the values.
- `max_messages as usize` survives in eight crates (BE, FI, FO, KE, PE, TE, XE, ZE). The rest switch to `usize::try_from` with a named
  invariant, or pass `u64` through.
