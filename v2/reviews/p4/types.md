# p4 / types

## Method

No `BASE` directory, so each crate was scored as a whole. Only `src/` was read (tests are irrelevant to this dimension). I ran the same steps on every crate:

1. Listed every `struct` / `enum` / `type` definition in `src/*.rs`.
2. Grepped for struct fields and parameters typed `String`, `&str`, `u64`, `u32`, `usize`, `Option<String>`, `HashMap<String,..>`, `Value` or `Map<String, ..>`.
3. Grepped for string comparisons and dispatch (`== "`, `.as_str() {`), handle/id formatting and parsing (`"r-"`, `"m-"`, `strip_prefix`), and sentinels (`::MAX`, `unwrap_or(0)`, `String::new()`, `: 0,`).
4. Counted `Value` / `Map<String` uses per file, then opened the ones outside the parsing module.
5. Checked how FIFO fields, the dead-letter pair, `Fifo`/`ContentBasedDeduplication` and the message delivery state are modelled (`grep fifo: bool|group_id: Option|sequence_number: Option|receipt: Option|visible_at|invisible_until`).
6. Read the request/command enum, the queue attribute struct, and the `Message` struct with its state type in every crate.

Note: request-side queue names kept as `String` were not held against a crate. The spec says any string is a valid lookup key ("whatever the string looks like"), so a raw string there is a fair choice. A receipt handle parsed in the broker rather than the parser was also accepted, because the spec checks validity after the queue lookup.

## Scores

| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| PA | 4 | 1 | PA/src/queue.rs:172 (`Message` with `Placement` enum + `Option<Reception>` coupling count, first-received and latest delivery) | PA/src/queue.rs:118 (dedup lookup takes `&str`, not `DedupKey`) | Newtypes for every id, `Body`, `QueueRef`, `Clock`/`Seconds`. `Kind::Fifo(FifoState)`. Handle parsed to `Option<ReceiptHandle>` at the boundary (request.rs:51) |
| MU | 4 | 1 | MU/src/queue.rs:31 (`QueueKind::Fifo { content_based_deduplication }`) and queue.rs:78 (`Hold` ties handle to in-flight) | MU/src/request.rs:55 (`handle: String`; body stays `String` at request.rs:68) | Timestamp/Seconds/QueueName/GroupId/DeduplicationKey/SequenceNumber types. Fields that belong together are one enum |
| WE | 4 | 1 | WE/src/store.rs:32 (`Delivery::Pending/Received` holds count, first-received, handle, visible_from together) | WE/src/request.rs:55 (`receipt_handle: String` until broker.rs:124) | `Body`, `MaxReceiveCount`, `Timestamp`, `Kind::Fifo(FifoState)`, `FifoTag`. Almost no bare primitives past the parser |
| QI | 3 | 4 | QI/src/queue.rs:37 (`fifo: Option<FifoPosition>` with `GroupId` + `SequenceNumber`) | QI/src/attributes.rs:138 (`fifo: bool` and `content_based_deduplication: bool` as independent fields) | Ids, `Instant`, `Seconds` and `DedupKey` all typed. Delayed vs in-flight is inferred from `receive_count == 0` (queue.rs:100), not from a state type |
| XI | 3 | 4 | XI/src/queue.rs:102 (`Hold::{None, Delayed, InFlight{receipt}}`) | XI/src/request.rs:85 (`deduplication_id: Option<String>` while group is `GroupId`) | Typed ids and `QueueName` all the way down. Times, sequence numbers and arrivals stay bare `u64` (queue.rs:92, 75) |
| ZU | 3 | 4 | ZU/src/queue.rs:384 (`Hold` enum) and types.rs:12 (`Time`) | ZU/src/queue.rs:24 (stored `DeadLetter.queue: String` though `QueueName` exists) | Good core model. One `Tag` type serves both group and dedup ids. Sequence stays `u64`, durations `u32` |
| QE | 2 | 7 | QE/src/types.rs:109 (`ReceiptHandle`) and attributes.rs:20 (`QueueKind`) | QE/src/queue.rs:262 (`invisible_until: 0` sentinel next to a separate `receipt: Option`) | Id newtypes exist, but times are bare `u64`, FIFO fields are two Options (queue.rs:87-88), and responses are a `Map<String, Value>` (broker.rs:19) |
| FU | 1 | 8 | FU/src/queue.rs:67 (`Visibility` enum ties receipt to in-flight) | FU/src/queue.rs:88 (`group_id: Option<String>` and `sequence_number: Option<u64>` split; id `u64`) | No id newtypes. Queue map keyed by `String`, handles and ids as `u64`, `fifo: bool` |
| KE | 1 | 8 | KE/src/queue.rs:131 (`FifoFields` couples group and sequence) | KE/src/queue.rs:147 (queue builds `Delivery { message_id: String, receipt_handle: String }` by formatting inside the domain) | Bare `u64`/`String` everywhere. Wire formatting has leaked into the queue |
| NI | 1 | 8 | NI/src/queue.rs:34 (`Receipt { handle, visible_at }`) | NI/src/broker.rs:22 (`Output = Map<String, Value>` built by hand for every command) | Bare ids and times, FIFO fields split (queue.rs:49-50), untyped JSON output inside the broker |
| TA | 1 | 8 | TA/src/queue.rs:33 (`State` enum) | TA/src/queue.rs:43 (`group_id: Option<String>`, `sequence_number: Option<u64>` split) | Typed `Command`, but all ids, handles and names are bare `u64`/`String`. Receipt validity is checked by re-formatting (broker.rs:291) |
| SO | 0 | 12 | SO/src/attributes.rs:19 (`DeadLetterPolicy` struct) | SO/src/broker.rs:76 (command dispatched by `request.command.as_str()`; raw `Map<String, Value>` params go into every handler) | No typed command, no id types. Parsing happens inside the handlers, and `invisible_until: 0` / `delayed_until = 0` are sentinels (queue.rs:54, 66) |

## What separates the top from the bottom

- **Typed command at the boundary.** At one extreme, PA turns the line into a fully typed `Command` that already holds `QueueRef`, `Body`, `Option<ReceiptHandle>` (PA/src/request.rs:15-58). At the other, SO keeps `command: String` plus a raw `Map<String, Value>` and parses inside each handler (SO/src/protocol.rs:16-17, broker.rs:76).
- **Ids and handles.** The top six define `MessageId`/`ReceiptHandle`/`SequenceNumber` with `Display` and a strict `parse` (e.g. WE/src/types.rs:202-245). FU, KE, NI, TA and SO pass `u64` and format `"m-{}"`/`"r-{}"` at the call site (NI/src/broker.rs:459-469, TA/src/protocol.rs:111-115).
- **Delivery state as one value.** MU, WE, XI and ZU make a `Hold`/`Delivery` enum that carries the receipt handle only in the in-flight variant (WE/src/store.rs:32), and PA does the same with `Option<Reception>`. QE and SO keep `invisible_until: u64` with `0` as "never delivered", next to a separate `receipt: Option` (QE/src/queue.rs:84-91).
- **Queue kind.** MU folds `ContentBasedDeduplication` into `QueueKind::Fifo` (MU/src/queue.rs:31). PA, WE and ZU put the FIFO-only state into `Kind::Fifo(..)`. Every crate from QI down stores `fifo: bool` and `content_based_deduplication: bool` side by side (QI/src/attributes.rs:138, FU/src/queue.rs:48).
- **FIFO message fields.** Top crates use one `Option<FifoTag>` (XI/src/queue.rs:95, WE/src/store.rs:26). FU, NI, QE, SO and TA keep two parallel Options that can disagree (QE/src/queue.rs:87-88).
- **Output shape.** QI, WE, PA, MU, XI and ZU serialize typed result structs. NI, QE, KE and SO assemble `Map<String, Value>` inside the broker or service (NI/src/broker.rs:22, QE/src/broker.rs:19).

## Patterns seen in most crates

- Every crate models the dead-letter pair as one `Option<DeadLetter{queue, max_receive_count}>` in stored attributes. On the request side they keep two Options and combine them in a checked accessor. That is reasonable, since the spec makes "exactly one present" an error.
- Every crate has a closed `ErrorCode` enum, and all but SO dispatch on a typed command or request enum. Only SO compares command names as strings after the parser.
- Request queue names are `String` in nearly all crates, which is reasonable under the spec. Only PA (`QueueRef`) and XI (unvalidated `QueueName`) give lookup names a type.
- Durations and counts (`max_messages`, `receive_count`, attribute seconds) stay bare `u32`/`u64` everywhere except MU, PA, QI and WE (`Seconds`).
