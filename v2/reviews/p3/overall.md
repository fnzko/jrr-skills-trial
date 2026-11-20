# p3 / overall

## Method

I read every file in `BASE/src/` in full. For each of the 18 attempts I read the complete
`diff -r -U1 BASE <CODE>` (src and tests), plus any new source file in full (`ids.rs` in CU, QO and
SI, `time.rs` in CU). I asked the same questions of every diff: did the change remove the
`require` + `.expect("required")` coupling; did it turn stringly or bare-integer data into types
(command, ids, time, queue name, FIFO fields, dead-letter pair); did it remove duplicated checks
(the `ContentBasedDeduplication` rule, the in-flight lookup that ran twice); did it add helpers,
structs or comments that carry no weight; did it tighten tests. Line numbers come from a per-crate
grep for the same set of patterns. No crate was built or run.

BASE is already a sound crate, so the attempts are judged on how far they move it. Most diffs are
small (40 to 210 changed lines). Only CU (840), SI (702) and QO (476) restructure anything.

## Scores

| code | score | rank | best evidence | worst evidence | reason |
|---|---|---|---|---|---|
| CU | 4 | 1 | CU/src/attributes.rs:38 (`QueueKind` makes "dedup on a standard queue" unrepresentable; plus `QueueName` CU/src/naming.rs:20) | CU/src/broker/message_commands.rs:40 (`received_json` still takes a `fifo` bool next to `Option<FifoPosition>`) | Parses at the boundary into real types (Command, QueueName, Instant/Seconds, ids, DeadLetterUpdate); larger but each piece is small and standard |
| SI | 4 | 2 | SI/src/request.rs:168 (`require` returns the typed `[Param; N]`, so `.expect("required")` is gone by construction) | SI/src/attributes.rs:24 (`Arc<str>` for the DLQ name only to dodge one clone) | Delivery enum, NewMessage, Command, ids; slightly clever in places (`static NULL` placeholder) |
| QO | 3 | 3 | QO/src/request.rs:17 (Command enum; also `FifoPlacement` QO/src/queue.rs:23) | QO/src/broker/message_commands.rs:96 (`fifo: None` placeholder filled in later by `push`) | Good id and FIFO types, but `Duration` used for instants (QO/src/broker.rs:25) needs a comment to be understood |
| ME | 3 | 4 | ME/src/request.rs:82 (`required_string` replaces every `.expect`) | ME/src/broker/message_commands.rs:53 (still calls `require` then `required_string`, two checks of one fact) | Best of the light cleanups: shared ranges, cast removed, tests compare parsed JSON |
| FE | 3 | 5 | FE/src/broker/message_commands.rs:197 (lookup returns queue + key, no second lookup) | FE/src/attributes.rs:76 (`apply(update, dead_letter)` ignores two fields of `update`) | Removes the double lookup and the expects; awkward `apply` signature |
| ZA | 3 | 5 | ZA/src/broker/message_commands.rs:207 (pure `in_flight_message` over a `&Queue`) | ZA/src/broker/message_commands.rs:179 (now/issued/lookup preamble copied into two handlers) | Same moves as FE with a small copy-paste cost |
| TE | 3 | 7 | TE/src/request.rs:88 (`required_string`/`required_integer`, `as usize` cast gone) | TE/src/broker/message_commands.rs:52 (`require` kept next to `required_*`) | Removes the panics with a minimal API change |
| PI | 3 | 8 | PI/src/request.rs:78 (`required_string`) | PI/src/attributes.rs:77 (`dead_letter_queue: _` in `apply`) | Same as TE plus the awkward split `apply` |
| NU | 2 | 9 | NU/src/broker/message_commands.rs:40 (`fifo_ids` returns `Option<FifoIds>`, duplicate check stays inline) | NU/src/broker/message_commands.rs:91 (`.expect("required")` untouched) | Clean FIFO extraction and error constructors; core fragility left |
| SU | 2 | 10 | SU/src/broker/message_commands.rs:67 (named `FifoIds` instead of a tuple) | SU/src/broker/message_commands.rs:92 (`.expect("required")` untouched) | Helper plus enum for one call site; little net gain |
| JI | 2 | 11 | JI/tests/binary.rs:29 (exact response line instead of `contains`) | JI/src/broker/message_commands.rs:54 (`.expect("required")` untouched) | Shared constants, dedup helper, tighter tests |
| HO | 2 | 12 | HO/src/attributes.rs:135 (dedup rule in one place on `AttributeUpdate`) | HO/src/broker/message_commands.rs:54 (`.expect("required")` untouched) | Constants and one shared check |
| VA | 2 | 12 | VA/src/attributes.rs:9 (shared range bounds) | VA/src/broker/message_commands.rs:54 (`.expect("required")` untouched) | Same as HO; the check lives as an `impl` in the wrong module (VA/src/broker/queue_commands.rs:20) |
| FO | 2 | 12 | FO/src/broker/queue_commands.rs:18 (shared dedup check) | FO/src/broker/message_commands.rs:26 (`check_body_len`, a one-use helper) | Same as HO plus a single-use helper |
| PO | 2 | 15 | PO/src/error.rs:63 (error constructors in one place) | PO/src/broker/message_commands.rs:57 (`resolve_fifo_ids` with six positional parameters) | Moves send logic out but not into a simpler shape |
| VO | 2 | 15 | VO/src/error.rs:67 (`queue_does_not_exist` beside the other constructors) | VO/src/broker/message_commands.rs:48 (single-use `Send` struct to pass four args) | Adds `Send` + `FifoIds` + `Admission` around one call |
| WO | 2 | 17 | WO/src/request.rs:49 (one `missing_parameter`) | WO/src/broker/queue_commands.rs:42 (dedup rule still duplicated at :95) | Cosmetic constants only |
| BASE | 2 | 18 | BASE/src/request.rs:69 (clear `require`-then-read check order, documented) | BASE/src/broker/message_commands.rs:57 (`.expect("required")` relies on a call-order contract; string dispatch BASE/src/broker.rs:65) | Readable and correct, but untyped ids and time, duplicated rules |
| FI | 2 | 18 | FI/src/error.rs:58 (shared `invalid_value`) | FI/src/queue.rs:112 (doc comments that restate names, e.g. "The number of messages held") | Small gains offset by comment noise and a six-arg `check_fifo` |

## What separates the top from the bottom

- **Parsing required parameters into values.** CU (CU/src/request.rs, `Required`) and SI
  (SI/src/request.rs:168) make `require` return the values, so no handler can read a parameter it
  forgot to require. TE/ME/FE/ZA/PI swap `.expect` for `required_*`, which is safe but keeps two
  checks of one fact. The other ten keep `.expect("required")` at every handler
  (e.g. NU/src/broker/message_commands.rs:91).
- **Types for the domain.** Only CU, SI and QO add `MessageId`/`ReceiptHandle`/arrival-index
  newtypes and a `Command` enum. CU goes furthest with `QueueName` (keyed map with `Borrow<str>`)
  and `Instant` vs `Seconds`; QO's `Duration` for instants is a weaker choice
  (QO/src/broker.rs:25).
- **Fields that belong together.** CU, SI and QO fold `group_id` + `sequence_number` into one
  `Option<FifoPosition>`, and CU and SI turn the dead-letter pair into an enum. SI's `Delivery` enum
  (SI/src/queue.rs:24) ties receive count, first receive and handle together.
- **Removing duplicate work, not just duplicate text.** FE and ZA remove the second queue lookup in
  `DeleteMessage`/`ChangeMessageVisibility` (FE/src/broker/message_commands.rs:197), and so do CU
  and QO. Most of the rest only extract the repeated `ContentBasedDeduplication` check.
- **Extraction that adds ceremony.** PO, VO, FI and SU move the FIFO send rules into a helper with
  5 to 6 positional parameters, or a single-use struct (VO/src/broker/message_commands.rs:48). That
  helps the reader very little over BASE's inline block.

## Patterns seen in most crates

- Nearly every attempt names the shared range bounds (visibility, delay) and gathers the
  `MissingParameter` constructor in one place. Both are safe and add little.
- 9 of 18 make the `ContentBasedDeduplication` rule a single helper. Only CU also makes it a type
  invariant of the stored attributes.
- Most leave the stringly `dispatch`, the bare `u64` ids and clock, and the `Option` pair for FIFO
  fields as they were in BASE. 10 of 18 leave every `.expect("required")` in place.
- Test changes are rare and small: mostly replacing `is_ok()`/`contains` with exact values
  (JI, ME, TE, ZA, PI, FE, CU). No attempt makes the code harder to change safely. The spread runs
  from about BASE to clearly better, which is why no crate scores below 2.
