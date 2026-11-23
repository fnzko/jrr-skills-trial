# p7r / types

## Method

Read SPEC.md and all of `BASE/src` in full. For each of the 21 attempts I produced `diff -ruN BASE/src <CODE>/src`
(tests/ is identical to BASE in every attempt, so only src was compared) and read every added line of the diff,
skipping only lines that were blank, comments, or the eleven repeated `Command::X` variant/match arms. On top of
that, the same grep checklist ran over every crate's `src/`: `enum Command` / `pub command: String`, the string
`"CreateQueue" =>` match, newtype structs over `u64`/`String` (MessageId, ReceiptHandle, arrival key,
SequenceNumber, QueueName, time), `handles: HashMap<_, u64>`, `expect("required")` / `expect("presence...")`,
anonymous `(&str, &str)` tuples, `dead_letter_queue: Option<String>` beside a separate `max_receive_count`,
`max_messages as usize`, and `pub fifo: bool`. Scores are relative to BASE: BASE = 1 (stringly commands, bare
`u64` for three kinds of id, required parameters unwrapped with `expect`, dead-letter pair split in the update).

## Scores

| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| JA | 4 | 1 | JA/src/attributes.rs:38 (`QueueKind::Fifo { content_based_deduplication }`) | JA/src/broker/queue_commands.rs:105 (`DeadLetter.queue` still `String`, compared to name) | QueueName key, QueueKind, Instant/Seconds, all ids, DeadLetterUpdate, typed `Required` params |
| HO | 4 | 2 | HO/src/queue.rs:24 (`Delivery::Never / Delivered{count, first_at, handle}`) | HO/src/attributes.rs:24 (`queue: Arc<str>`, no QueueName type) | Delivery and DeadLetterUpdate enums bind co-dependent fields; all counters typed, `require` returns `Param`s |
| GU | 3 | 3 | GU/src/queue.rs:23 (`FifoPlacement { group_id, sequence_number }`) | GU/src/broker/message_commands.rs:88 (`fifo_ids.map(\|(group, _)\| ...)` tuple) | Command, private ids, ArrivalIndex, Duration clock; send ids still a tuple, `fifo: None` placeholder |
| ZE | 3 | 4 | ZE/src/broker/message_commands.rs:36 (`SendRules::{Standard, Duplicate, Fifo{..}}`) | ZE/src/broker/message_commands.rs:111 (enum collapsed back to `Some((g, d))`) | issuer types make "issued" handles typed; ArrivalIndex; relapses to a tuple |
| VO | 3 | 4 | VO/src/broker/message_commands.rs:37 (`SendTarget::Standard / Fifo{..}`) | VO/src/queue.rs:86 (`HashMap<ReceiptHandle, u64>` arrival key bare) | closed send kinds and FifoPlace; arrival index left as `u64` |
| VI | 3 | 6 | VI/src/broker/message_commands.rs:32 (`SendRequest::parse` at the boundary) | VI/src/ids.rs:8 (`MessageId(pub u64)` public field) | Command, ids incl. ArrivalKey, parsed request struct; ids constructible from any u64 |
| WE | 3 | 6 | WE/src/ids.rs:57 (`ArrivalKey` private with `FIRST/next`) | WE/src/attributes.rs:75 (`apply(&update, dead_letter)`: pair split, then passed beside the update) | Command, encapsulated ids, FifoSend struct; dead-letter pair still split in AttributeUpdate |
| LI | 3 | 8 | LI/src/ids.rs:26 (`ReceiptHandle(u64)` private, `parse/next`) | LI/src/queue.rs:102 (`HashMap<ReceiptHandle, u64>`) | Command and encapsulated ids; arrival key bare, FIFO ids a tuple |
| JO | 3 | 9 | JO/src/ids.rs:38 (`ArrivalIndex`, all three ids typed) | JO/src/request.rs:51 (`command: String`, parsed only at JO/src/broker.rs:65) | enum exists but the request still stores the string and parses it late |
| DA | 2 | 10 | DA/src/broker/message_commands.rs:52 (named `FifoIds`) | DA/src/broker/message_commands.rs:173 (`max_messages as usize`, ids bare u64) | Command + required getters + named FIFO ids; no id types |
| VA | 2 | 10 | VA/src/queue.rs:17 (`ArrivalIndex(u64)` private) | VA/src/broker.rs:66 (`"CreateQueue" =>` string dispatch kept) | good id types, but the command set stays stringly |
| CA | 2 | 12 | CA/src/request.rs:16 (`enum Command`) | CA/src/attributes.rs:110 (`dead_letter_queue: Option<String>` beside `max_receive_count`) | Command + required getters; ids and dead-letter pair unchanged |
| ZO | 2 | 12 | ZO/src/request.rs:16 (`enum Command`, parsed in `Request::parse`) | ZO/src/attributes.rs:110 (split dead-letter pair) | same as CA; also removed the `as usize` cast |
| RU | 2 | 12 | RU/src/request.rs:16 (`enum Command`) | RU/src/broker/message_commands.rs:60 (`Option<(&str, &str)>`) | Command + required getters; anonymous FIFO tuple |
| CE | 2 | 15 | CE/src/ids.rs:18 (`ReceiptHandle::parse` returns a typed handle) | CE/src/request.rs:16 (`command: String`, matched at CE/src/broker.rs:66) | ids typed, but command stringly and 11 `expect("required")` remain |
| WO | 2 | 16 | WO/src/ids.rs:17 (`ReceiptHandle` newtype with `parse`) | WO/src/broker.rs:66 (string dispatch) and WO/src/broker.rs:88 (`expect` after `require`) | ids typed, arrival key bare, commands stringly |
| XU | 2 | 17 | XU/src/request.rs:16 (`enum Command` parsed at the boundary) | XU/src/broker/queue_commands.rs:21 (`expect("required")`) | only the command enum changed |
| RO | 2 | 17 | RO/src/request.rs:16 (`enum Command`) | RO/src/broker.rs:84 (expects reworded, not removed) | command enum only; FIFO ids an anonymous tuple |
| SA | 1 | 19 | SA/src/broker/message_commands.rs:53 (named `FifoIds`) | SA/src/request.rs:16 (`command: String`) | BASE plus one named struct |
| BASE | 1 | 20 | BASE/src/error.rs:7 (`ErrorCode` closed enum) | BASE/src/broker.rs:66 (commands matched as strings) | stringly commands, bare u64 ids, split dead-letter update |
| NE | 1 | 20 | NE/src/broker/message_commands.rs:48 (`FifoSend::Duplicate/New`) | NE/src/broker/message_commands.rs:52 (`New(Option<(&str, &str)>)`) | no type changes beyond an enum around a tuple |
| HE | 1 | 20 | HE/src/broker/message_commands.rs:53 (`FifoSend` enum) | HE/src/broker/message_commands.rs:57 (anonymous tuple), HE/src/broker.rs:86 (`expect("required")`) | same as NE |

## What separates the top from the bottom

- Binding fields that only make sense together: only JA and HO replaced BASE's split `dead_letter_queue` /
  `max_receive_count` update fields with a three-state enum (JA/src/attributes.rs:128, HO/src/attributes.rs:96).
  HO also turned `receive_count` / `first_received_at` / `handle` into one `Delivery` enum (HO/src/queue.rs:24), and
  JA folded `fifo` + `ContentBasedDeduplication` into `QueueKind` (JA/src/attributes.rs:38). Every other crate keeps
  `pub fifo: bool` beside `content_based_deduplication: bool`.
- Identifier newtypes: the top half wraps MessageId, ReceiptHandle and the arrival key, with private fields and
  `FIRST/next` or issuer methods (GU/src/ids.rs:8, ZE/src/ids.rs:37). The bottom keeps the three as bare `u64`, which can be
  swapped without a compile error (BASE/src/queue.rs:16-28).
- Command parsing at the boundary: 15 crates added `enum Command`, but JO keeps `command: String` and parses it in
  dispatch (JO/src/request.rs:51). CE, VA, WO, SA, NE and HE still match command names as strings.
- Required parameters: crates that added `required_string`/`required_integer` or a typed `Required`/`Param` value
  (JA/src/request.rs:88, HO/src/request.rs:90) removed the `require` then `expect` pattern. RO and WO only reworded
  the `expect` (RO/src/broker.rs:84).
- Tuple relapse: GU, ZE, RU, RO, CE, NE and HE pass the FIFO group and deduplication ids as `(&str, &str)`. ZE builds a good enum
  and then turns it back into a tuple (ZE/src/broker/message_commands.rs:111).

## Patterns seen in most crates

- Good everywhere: `ErrorCode` is a closed enum, message `State` is an enum, "unset" is `Option` (no sentinels),
  and the dead-letter configuration on a queue is `Option<DeadLetter>`.
- Bad nearly everywhere: queue names stay `String`/`&str` through the whole broker (only JA has `QueueName`), so
  `DeadLetter.queue` is compared to names as strings in all 22. Results are still built as `Map<String, Value>`
  (`Members`) in every crate. Attribute names stay a `&str` list checked with `reject_unknown`, and no crate made them an enum.
  The `GroupId` and deduplication id stay plain strings.
