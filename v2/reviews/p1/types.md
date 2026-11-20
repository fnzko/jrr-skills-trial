# p1 / types

## Method
For each of the 15 crates I ran the same searches over `src/` and read the same things:
1. every `struct`/`enum`/`type` definition (one grep, all crates);
2. the full field lists of the domain model files (`queue.rs`, `attributes.rs`/`config.rs`, receipt/id modules);
3. struct fields typed as `String`/`&str`/`u64`/`u32`/`Option<...>` for ids, names, handles, groups, dedup ids, sequences;
4. string-literal matches/comparisons (`"Command" =>`, attribute names, `ends_with(".fifo")`, `strip_prefix("r-")`);
5. `serde_json::Value`/`Map<String, Value>` outside the request-parsing module;
6. sentinels (`u64::MAX`, `unwrap_or(0)`, `unwrap_or_default()`).
Where a crate had no typed command enum (BO, PO) I read the dispatch and one handler to confirm. No crate was read beyond these. Tests were excluded. No BASE directory exists.

## Scores
| code | score | rank | best evidence | worst evidence | reason |
|---|---|---|---|---|---|
| QO | 4 | 1 | QO/src/queue.rs:113 (`Availability::InFlight { until, handle }`) | QO/src/queue.rs:95 (dedup map keyed by `String` though `DeduplicationId` exists at types.rs:273) | Newtypes for every id, name, body, instant; ranged attribute types; `Kind`/`QueueType` enums; typed responses |
| FU | 4 | 2 | FU/src/queue.rs:22 (`QueueKind::Fifo { content_based_deduplication }`) | FU/src/queue.rs:106 (`handle: Option<ReceiptHandle>` kept apart from `availability` at :104) | `Bounded<MIN,MAX>`, `Body`, `DedupKey`, `QueueName` keys, typed output; one split state |
| HE | 4 | 3 | HE/src/queue.rs:76 (`Hold` enum carries handle with in-flight deadline) | HE/src/queue.rs:198 (`Received.delivered: Vec<usize>` indices; also `Sent.accepted: bool` :191) | `Ranged`, `Instant`/`Seconds`, `CommandName` enum, `Kind::Fifo(FifoState)`; body stays `&str` |
| FI | 3 | 4 | FI/src/domain.rs:91 (`QueueName`, plus macro newtypes for GroupId/DeduplicationId :148) | FI/src/attributes.rs:20 (all attributes bare `u64`, ranges only checked in parser) | Good id newtypes and typed output; attributes and times untyped, `hidden_until` split from `handle` |
| TO | 2 | 5 | TO/src/queue.rs:94 (`Hold { Delayed, InFlight { until, handle } }`) | TO/src/queue.rs:137 (`FifoSend { group_id: String, deduplication_id: String }`) | Typed ids and replies, good state enum; groups, sequences, attributes bare |
| TA | 2 | 6 | TA/src/queue.rs:40 (`State::Delayed(Time)`/`InFlight(Time)`) and ids.rs:45 `GroupId` | TA/src/broker.rs:17 (handlers build `Map<String, Value>` responses) | Group/dedup newtypes, `Admission` enum; `Time`/`Arrival` only aliases, sequence `u64` |
| BI | 2 | 7 | BI/src/queue.rs:58 (`Delivery { handle, invisible_until }` held as `Option`) | BI/src/queue.rs:189 (`oldest_sent_at: u64::MAX` sentinel for empty) | Typed request/response, FIFO state as `Option<FifoState>`; message id, handle, group raw |
| LI | 2 | 8 | LI/src/queue.rs:15 (`InFlight { until, handle }`) | LI/src/queue.rs:28 (`group_id: Option<String>` and `sequence: Option<u64>` separate though only valid together) | MessageId/ReceiptHandle newtypes; FIFO placement split; Value responses |
| CE | 2 | 9 | CE/src/ids.rs:15 (`ReceiptHandle` newtype) | CE/src/queue.rs:27 (`invisible_until` apart from `handle: Option<ReceiptHandle>` at :31) | Id newtypes and `FifoPlacement`; split in-flight state, Value responses, `pub u64` inners |
| WU | 2 | 10 | WU/src/queue.rs:28 (`Hold` enum with handle) | WU/src/queue.rs:89 (`Sent.duplicate: bool` flag instead of an outcome enum) | Good state enum, QueueName for create; group/dedup as `String`, Value output |
| ZE | 2 | 11 | ZE/src/queue.rs:43 (`InFlight { receipt, until }`) | ZE/src/queue.rs:88 (`sequence: u64` always set, emitted only behind a `fifo` bool, broker.rs:96; plus `accepted: bool`) | Three id newtypes; sequence overloaded as arrival key for standard queues |
| MI | 1 | 12 | MI/src/queue.rs:131 (`SendOutcome::{Accepted, Duplicate}`) | MI/src/queue.rs:98 (`handle: Option<u64>` apart from `in_flight_until: u64` at :96) | No id newtypes at all; handles parsed to `u64` in broker; Value responses |
| VU | 1 | 13 | VU/src/attributes.rs:19 (`dead_letter: Option<DeadLetter>`) | VU/src/queue.rs:96 (`HandleLog::lookup(&str)` parses the raw handle inside the queue, returns `u64`) | No newtypes, handle text travels inward, `u64::MAX` sentinel (queue.rs:126) |
| BO | 0 | 14 | BO/src/queue.rs:48 (`dead_letter: Option<DeadLetter>`) | BO/src/broker.rs:57 (dispatch on command string with raw `Params`; each handler parses its own input) | No command type, no id types, handlers return `Map<String, Value>`, `unwrap_or(0)` after `require` (broker.rs:90) |
| PO | 0 | 14 | PO/src/attributes.rs:33 (`dead_letter: Option<DeadLetter>`) | PO/src/broker.rs:65 (`match req.command.as_str()` passing raw `Params` into handlers) | Same shape as BO: parsing inside handlers, all ids `u64`, Value output |

## What separates the top from the bottom
- Parse at the boundary: QO, FU, HE, FI build a typed command enum whose payloads are already validated (`QueueName`, ranged integers, `GroupId`); BO (broker.rs:57) and PO (broker.rs:65) keep a raw `Map<String, Value>` and parse in every handler, so `expect("required parameter is present")` appears inside business logic (BO/src/queue_commands.rs:20).
- Ranged attribute types: FU/src/scalar.rs:7, HE/src/values.rs:14 and QO/src/types.rs:49 make `VisibilityTimeout` etc. distinct types; everyone else stores `u32`/`u64` and relies on the parser having checked the range.
- Queue kind as an enum: FU/src/queue.rs:22, HE/src/queue.rs:41, QO/src/queue.rs:27 put `ContentBasedDeduplication` inside the FIFO variant; the rest keep `fifo: bool` plus `content_based_deduplication: bool`, allowing the forbidden combination.
- In-flight state with its handle: HE, QO, TO, WU, LI, ZE, BI hold the deadline and the handle in one variant/struct; CE, FI, FU, MI, PO, VU, BO keep `invisible_until` and `handle: Option<_>` as independent fields.
- Id newtypes: QO/FU/HE/FI type every identifier; MI, VU, BO, PO use bare `u64` for message ids and receipt handles throughout.
- Typed responses: BI, FI, FU, HE, QO, TO serialize response structs; the other nine assemble `serde_json::Value` objects inside the service.

## Patterns seen in most crates
- Command names and attribute names are matched as strings only at the parser (fine), and none compares error codes or states as strings: all use `ErrorCode` and a `State` enum.
- Dead-letter configuration is `Option<DeadLetter { queue, max_receive_count }>` in every crate, so the "both or neither" rule is a type everywhere.
- The deduplication map is keyed by `String` in 13 of 15 crates, even where a `DeduplicationId` type exists (only FU `DedupKey` and HE `DeduplicationId` key it by type).
- Queue names in non-create commands stay `String`/`&str` nearly everywhere; defensible, since the spec looks them up "whatever the string looks like".
- Receipt handles are kept as text until after the queue lookup in all crates (needed for check order), then parsed with `strip_prefix("r-")`; only the top crates parse into a newtype.
