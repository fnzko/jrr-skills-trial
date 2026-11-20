# p6r / types
## Method
- Read BASE/src in full (all 11 files) to know the starting point and its type debts.
- For each of the 18 attempts: `diff -r -u BASE/src <CODE>/src`, read every changed hunk, then read
  any new type module in full (`ids.rs`, `time.rs`, the new types in `queue.rs`/`attributes.rs`/`naming.rs`).
- The same greps in every crate: all `struct`/`enum`/`type` declarations; the dispatch line in
  `broker.rs` (string match vs `Command` enum); `Message` fields in `queue.rs` (id, handle, group id,
  sequence number); the dead-letter fields in `AttributeUpdate`; the key type of the queue map;
  `Map<String, Value>` uses. Tests were not scored.
- BASE debts every attempt could pay: command dispatched by string, message id / receipt handle /
  arrival key / sequence number all bare `u64`, FIFO-only fields as two loose `Option`s, the
  dead-letter pair as two loose `Option`s in the update, queue names and times as bare `String`/`u64`,
  results as `Map<String, Value>`. BASE already has good `ErrorCode`, `State`, `DeadLetter` + `Option`.

## Scores
| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| JI | 4 | 1 | JI/src/naming.rs:20 (`QueueName` newtype, parsed once, map key at JI/src/broker.rs:31) | JI/src/attributes.rs:31 (`DeadLetter.queue: String`, not `QueueName`) | Command enum, MessageId/ReceiptHandle/SequenceNumber/Arrival, `Instant`/`Seconds`, `QueueKind` enum, `FifoPosition`, `DeadLetterUpdate`; only small lapses |
| CE | 4 | 2 | CE/src/queue.rs:24 (`Delivery` enum: count, first-received and handle exist only together) | CE/src/broker.rs:29 (queue map keyed by bare `String`) | Command enum, four counter newtypes, `FifoPosition`, `Delivery`, `DeadLetterUpdate`; times stay `u64` |
| PI | 4 | 3 | PI/src/attributes.rs:96 (`DeadLetterParams` enum: Absent/Complete/Partial) | PI/src/queue.rs:37 (`sequence_number: u64` bare) | Command enum, three id newtypes, `FifoPosition`, dead-letter enum; times and names bare |
| NE | 3 | 4 | NE/src/queue.rs:23 (`FifoPlacement` groups group id + sequence) | NE/src/attributes.rs:103 (dead-letter pair still two loose `Option`s) | Command enum, id newtypes, FIFO grouping; `Duration` used for clock instants is a muddle |
| KU | 3 | 5 | KU/src/ids.rs:60 (`SequenceNumber` newtype) | KU/src/queue.rs:27 (`group_id`/`sequence_number` two loose `Option`s) | Command parsed at boundary, four id newtypes; FIFO fields and dead-letter pair not grouped |
| TE | 3 | 6 | TE/src/request.rs:16 (`Command` enum parsed in `Request::parse`) | TE/src/queue.rs:22 (FIFO fields loose, sequence `u64`) | Command enum plus MessageId/ReceiptHandle/ArrivalIndex; nothing grouped |
| MI | 3 | 7 | MI/src/ids.rs:28 (`MessageId` private-field newtype with `issue`) | MI/src/queue.rs:23 (`sequence_number: Option<u64>` loose) | Command enum and three id newtypes; FIFO and dead-letter fields untouched |
| PA | 3 | 8 | PA/src/ids.rs:17 (`ReceiptHandle` with `parse`) | PA/src/request.rs:51 (command kept as `String`, parsed late in `command()`) | ids typed, but the command is not parsed into the enum at the boundary |
| DE | 2 | 9 | DE/src/ids.rs:18 (`ReceiptHandle` newtype, handle map typed) | DE/src/broker.rs:65 (dispatch still `match req.command.as_str()`) | id newtypes (with `pub` fields) but command stays a string |
| BE | 2 | 10 | BE/src/request.rs:16 (`Command` enum) | BE/src/queue.rs:16 (`id: u64`; ids, handles, keys all bare) | only the command closed set fixed |
| FI | 2 | 10 | FI/src/request.rs:16 (`Command` enum) | FI/src/queue.rs:16 (`id: u64`; ids, handles, keys all bare) | same change as BE |
| PE | 1 | 12 | PE/src/broker/message_commands.rs:53 (`FifoIds` named struct + `FifoSend` enum) | PE/src/broker.rs:65 (string dispatch) | a local FIFO grouping; every BASE type debt remains |
| XE | 1 | 13 | XE/src/broker/message_commands.rs:53 (`FifoSend` enum) | XE/src/broker.rs:65 (string dispatch) | `FifoSend::New` carries a bare `(&str, &str)` tuple; nothing else typed |
| ZE | 1 | 13 | ZE/src/broker/message_commands.rs:48 (`FifoSend` enum) | ZE/src/broker.rs:65 (string dispatch) | same as XE |
| BASE | 1 | 15 | BASE/src/attributes.rs:33 (`dead_letter: Option<DeadLetter>`) | BASE/src/broker.rs:65 (`match req.command.as_str()`) | ids, handles, keys, sequence all bare `u64`; command by string; loose FIFO options |
| FO | 1 | 15 | FO/src/attributes.rs:33 (`Option<DeadLetter>`) | FO/src/broker.rs:65 (string dispatch) | no type change (adds `DeadLetter` arg to `apply`, keeps loose fields) |
| KE | 1 | 15 | KE/src/attributes.rs:33 (`Option<DeadLetter>`) | KE/src/broker.rs:65 (string dispatch) | no type change |
| PO | 1 | 15 | PO/src/attributes.rs:33 (`Option<DeadLetter>`) | PO/src/broker.rs:65 (string dispatch) | no type change |
| QO | 1 | 15 | QO/src/attributes.rs:38 (`Option<DeadLetter>`) | QO/src/broker.rs:65 (string dispatch) | no type change |

## What separates the top from the bottom
- Identifier newtypes: the top eight give message id, receipt handle and arrival key their own types
  (JI/src/ids.rs:8, CE/src/ids.rs:29), so `handles: HashMap<ReceiptHandle, Arrival>` can no longer be
  filled backwards; the bottom keep `HashMap<u64, u64>` (BASE/src/queue.rs:78).
- The command closed set: everyone from rank 1 to 8 except DE parses `Command` into an enum in
  `Request::parse`; PA parses it lazily (PA/src/request.rs:51), DE and the bottom eight still dispatch
  on a string (BASE/src/broker.rs:65).
- Fields that live together: JI, CE, PI, NE fold `group_id` + `sequence_number` into one
  `Option<FifoPosition>` (CE/src/queue.rs:49); JI, CE, PI also make the dead-letter pair an enum
  (JI/src/attributes.rs:128); CE goes further with `Delivery` (CE/src/queue.rs:24).
- Only JI types the remaining wide-open values: `QueueName` (JI/src/naming.rs:20), `Instant`/`Seconds`
  (JI/src/time.rs:7), and `QueueKind` that makes content-based dedup unrepresentable on a standard
  queue (JI/src/attributes.rs:38).
- Many low-ranked attempts spent their effort on error helpers and range constants
  (BE/src/limits.rs, ZE/src/error.rs) which do not touch the type model.

## Patterns seen in most crates
- All 19 keep results as `type Members = Map<String, Value>` and keep `Params` as a view over the raw
  JSON map passed into every handler instead of a typed request per command.
- All but JI keep queue names as bare `String` (including `DeadLetter.queue`), and all but JI and NE
  keep clock values as bare `u64`.
- Attribute names stay a `[&str; 7]` list checked by string (BASE/src/attributes.rs:8) in every crate.
- `ErrorCode`, `State` and `Option<DeadLetter>` are good in BASE and nobody regressed them.
