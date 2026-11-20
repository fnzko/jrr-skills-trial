# p3 / types

## Method

Read every file of `BASE/src` in full and scored it as a crate. For each of the 18 attempts, read the
full `diff -ruN BASE/src <CODE>/src` (every hunk, tests inside `src` included) and, for the three
attempts that added modules (CU, QO, SI), read the new files (`ids.rs`, `time.rs`, `naming.rs`) and
the changed `queue.rs`/`attributes.rs`/`request.rs` in full. Then ran the same grep over every crate
for the type markers of this dimension: how commands are dispatched (`req.command.as_str()` vs an
enum), the type of `Message::id`, the `handles` map key, `dead_letter_queue: Option<String>`, the
`(group, deduplication)` tuple, the `max_messages as usize` cast, and the type of `now`. Files under
`tests/` were not read; they do not carry domain types. Since every attempt starts from BASE, an
attempt that changed no types scores the same as BASE; ranks then order the attempts by how much
type work their diff added.

## Scores

| code | score | rank | best evidence | worst evidence | reason |
|---|---|---|---|---|---|
| CU | 4 | 1 | CU/src/attributes.rs:38 `QueueKind::Fifo { content_based_deduplication }` makes "dedup on a standard queue" impossible to represent | CU/src/attributes.rs:31 `DeadLetter::queue` is still a bare `String` next to the new `QueueName` | Validated `QueueName` (naming.rs:20), `Instant`/`Seconds`, `MessageId`/`ReceiptHandle`/`SequenceNumber`/`Arrival`, `Command` enum, `FifoPosition`, `DeadLetterUpdate` |
| SI | 4 | 2 | SI/src/queue.rs:24 `enum Delivery { Never, Delivered { receive_count, first_received_at, handle } }` keeps fields that only exist together in one variant | SI/src/queue.rs:38 time is still bare `u64` (`sent_at`, `now`) | Counter newtypes for all four id kinds (ids.rs), `Command` enum, `FifoPosition`, `DeadLetterUpdate`, typed `require` |
| QO | 3 | 3 | QO/src/ids.rs:30 `ReceiptHandle` newtype with `parse`/`Display` at the boundary | QO/src/broker.rs:24 one `Duration` "serves instants and spans"; queue.rs:25 `sequence_number: u64` | `MessageId`/`ReceiptHandle`/`ArrivalIndex`, `Command` enum, `FifoPlacement`; queue name and DLQ pair untouched |
| VO | 2 | 4 | VO/src/broker/message_commands.rs:62 `enum Admission { Standard, Fifo(FifoIds), Duplicate(Accepted) }` | VO/src/queue.rs:16 `id: u64`, handles/arrival keys all bare `u64` | Replaces the anonymous tuple with a named struct and a closed outcome enum; nothing else |
| SU | 2 | 4 | SU/src/broker/message_commands.rs:53 `FifoIds { group_id, deduplication_id }` + `FifoSend` enum | SU/src/queue.rs:78 `handles: HashMap<u64, u64>` | Same local improvement as VO |
| NU | 2 | 4 | NU/src/broker/message_commands.rs:33 named `FifoIds` replaces `(group, dedup)` tuple | NU/src/broker.rs:65 command dispatched by string match | Named struct only |
| FI | 2 | 7 | FI/src/broker/message_commands.rs:53 `FifoSend` enum separates duplicate from new | FI/src/broker/message_commands.rs:57 `New(Option<(&str, &str)>)` keeps the anonymous pair | Enum added, tuple kept |
| PO | 2 | 7 | PO/src/broker/message_commands.rs:48 `FifoSend` enum | PO/src/broker/message_commands.rs:52 `New(Option<(&str, &str)>)` | Same as FI |
| TE | 2 | 9 | TE/src/broker/message_commands.rs:149 `plan_receive` takes the parsed `u64`, removing the `as usize` cast | TE/src/queue.rs:16 `id: u64` | Only the cast removed |
| ME | 2 | 9 | ME/src/broker/message_commands.rs:150 `as usize` cast removed | ME/src/broker.rs:65 string command dispatch | Only the cast removed |
| ZA | 2 | 9 | ZA/src/broker/message_commands.rs:150 cast removed | ZA/src/attributes.rs:104 `dead_letter_queue: Option<String>` kept and ignored as `_` in `apply` (attributes.rs:74) | Cast removed; DLQ pairing passed alongside the raw pair instead of replacing it |
| PI | 2 | 9 | PI/src/attributes.rs:70 `apply` now receives a validated `Option<DeadLetter>` | PI/src/attributes.rs:100 raw `dead_letter_queue`/`max_receive_count` still on the struct, now dead in `apply` | Half step on the paired fields |
| FE | 2 | 9 | FE/src/attributes.rs:69 `apply` takes `Option<DeadLetter>` | FE/src/attributes.rs:99 raw pair kept and ignored | Same half step as PI |
| BASE | 2 | 14 | BASE/src/attributes.rs:33 `dead_letter: Option<DeadLetter>` models the both-or-neither pair | BASE/src/queue.rs:78 `handles: HashMap<u64, u64>`: receipt handle and arrival index both bare `u64` | Enums for `ErrorCode`/`State`/`Step`; bare ids, string command dispatch, tuples |
| WO | 2 | 14 | WO/src/attributes.rs:37 `Option<DeadLetter>` (inherited) | WO/src/queue.rs:78 `HashMap<u64, u64>` | No type changes (named constants only) |
| VA | 2 | 14 | VA/src/attributes.rs:39 `Option<DeadLetter>` (inherited) | VA/src/queue.rs:16 `id: u64` | No type changes |
| HO | 2 | 14 | HO/src/attributes.rs:42 `Option<DeadLetter>` (inherited) | HO/src/broker.rs:65 string command dispatch | No type changes |
| JI | 2 | 14 | JI/src/attributes.rs:37 `Option<DeadLetter>` (inherited) | JI/src/queue.rs:78 `HashMap<u64, u64>` | No type changes |
| FO | 2 | 14 | FO/src/queue.rs:32 `State` enum (inherited) | FO/src/queue.rs:16 `id: u64` | No type changes |

## What separates the top from the bottom

- Newtypes for the identifier kinds. CU, SI and QO give `MessageId`, `ReceiptHandle` and the arrival
  index their own types (QO/src/queue.rs:19, SI/src/ids.rs, CU/src/ids.rs). The receipt handle is
  parsed once into that type and rendered by `Display`, so `HashMap<ReceiptHandle, Arrival>` replaces
  BASE's `HashMap<u64, u64>` (BASE/src/queue.rs:78), where the two `u64`s can be swapped without
  the compiler noticing. No other attempt touched these.
- The command name is parsed into an enum at the boundary only in CU, SI and QO
  (`match req.command` at CU/src/broker.rs:68). The other 15 crates keep
  `match req.command.as_str()` (BASE/src/broker.rs:65).
- Making invalid states unrepresentable. CU's `QueueKind` (CU/src/attributes.rs:38) and SI's `Delivery`
  (SI/src/queue.rs:24) model rules from the spec in the type itself. CU, SI and QO also fold the FIFO
  pair `group_id`/`sequence_number` into one `Option<FifoPosition>`, where BASE has two independent
  `Option`s.
- Time. CU is the only crate that tells instants from spans (`Instant + Seconds`, CU/src/time.rs).
  QO uses `Duration` for both (QO/src/broker.rs:24). SI and every other crate keep bare `u64`.
- In the middle tier, VO, SU and NU replace the anonymous `(group, dedup)` tuple with a named
  `FifoIds`, and VO and SU add an outcome enum. FI and PO add the enum but keep
  `Option<(&str, &str)>` inside it. PI, FE and ZA pass the validated `Option<DeadLetter>` into `apply`
  but leave the raw pair on `AttributeUpdate` as dead `_` fields, which is worse than replacing it
  (CU and SI do replace it with `DeadLetterUpdate`).

## Patterns seen in most crates

- 15 of 18 attempts treated this pass as constants plus error helpers: named range constants,
  `Error::missing_parameter`, `required_string`. None of that changes a type. Their type design is
  BASE's: good closed enums for `ErrorCode`, `State` and `Step`, and `Option<DeadLetter>` on the
  stored attributes, but bare `u64` for every id and timestamp, and a bare `String` queue name.
- Every crate, the top three included, still threads `serde_json::Map` through handlers as
  `Params`, and builds results as `Map<String, Value>` (`Members`). Every crate also keeps
  deduplication ids and group ids as bare `String`.
- Even the best crates leave `DeadLetter::queue` as a plain `String` (CU/src/attributes.rs:31). In CU
  this is the case even though a validated `QueueName` type exists one module away.
