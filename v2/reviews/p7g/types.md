# p7g / types

## Method
Same procedure for every crate, no cargo, no BASE (none exists):
1. Extracted every `struct` / `enum` / `type` definition outside `#[cfg(test)]` (first ~15 lines of each) with a script and read all 21 dumps in full.
2. Ran the same greps over the non-test part of every source file: `json!(`, `Map<String, Value>` / `Members` aliases, `insert("...")`, string-literal comparisons (`== "`, `.as_str() ==`), command and attribute-name `match` arms, and where `r-` handles are parsed.
3. Spot-read the same kinds of spots in each crate where the dumps raised a question: the stored `Message` fields (state as enum vs raw timestamps), how the queue name for lookups is typed, and where request parameters become typed values.
I judged against the spec's closed sets (commands, attributes, error codes, message state), its identifiers (queue name, message id, receipt handle, group id, dedup id, sequence number) and its bounded integers.

## Scores
| code | score | rank | best evidence | worst evidence | reason |
|---|---|---|---|---|---|
| BO | 4 | 1 | BO/src/queue.rs:19 `Kind::Fifo { content_based_deduplication }` | BO/src/command.rs:18 `dead_letter_queue: Option<String>` | Own type per attribute, Body, DedupKey, Arrival; even error causes are typed sub-enums; only lookup/DLQ input stays a string |
| KE | 4 | 1 | KE/src/text.rs:92 `DedupKey` (plus `Bounded` aliases in scalar.rs) | KE/src/command.rs:51 `queue: String` | Every identifier and range typed, typed command at boundary; only the lookup name is a bare String |
| MO | 4 | 1 | MO/src/attributes.rs:103 `DeadLetterRequest { Absent, QueueOnly, CountOnly, Both }` | MO/src/protocol.rs:20 `queue: String` | Paired DLQ fields modelled as one enum, `HandleText` enum for handles, per-range types, Body/DedupKey |
| XI | 4 | 1 | XI/src/types.rs:146 `ReceiptHandle(NonZeroU64)` | XI/src/queue.rs:95 `HashMap<String, DeduplicationEntry>` | Const-generic seconds/counts, `Kind::Fifo(FifoState)` keeps FIFO-only state inside the variant; dedup key left as String |
| FO | 3 | 5 | FO/src/types.rs:11 `Bounded<MIN, MAX>` | FO/src/fifo.rs:31 `sequence_number: u64` | Strong newtypes and DedupKey; sequence numbers and response attribute view fall back to raw ints |
| ZU | 3 | 5 | ZU/src/attributes.rs:23 `DeadLetter { QueueName, MaxReceiveCount }` | ZU/src/queue/mod.rs:62 `handle: Option<ReceiptHandle>` beside `State::InFlight` | Typed throughout; handle lives outside the in-flight state it belongs to, `Sent.duplicate: bool` |
| LO | 3 | 5 | LO/src/queue.rs:170 `enum Kind { Standard, Fifo(FifoState) }` | LO/src/queue.rs:198 `delivered: Vec<usize>` | Ranged types, dedup keyed by type; body String, `receive_count: u64`, index lists and `accepted: bool` flags |
| VU | 3 | 5 | VU/src/request.rs:15 `DeadLetterRequest { Unset, Complete, Incomplete }` | VU/src/types.rs:176 `QueueName::new(String)` with no validation | Good request modelling; QueueName carries no invariant and dedup map is `HashMap<String,_>` |
| ZE | 3 | 5 | ZE/src/types.rs:153 `DeduplicationKey` | ZE/src/queue.rs:45 `receive_count: u64` (body String at :43) | Bounded attrs, Phase enum, typed keys; body and counts raw, reply view is raw u32 |
| JU | 3 | 5 | JU/src/queue.rs:136 `enum Deliveries { Never, Some { count, first_at } }` | JU/src/time.rs:9 `Seconds(pub u32)` used for every range | Nice paired-field enums and DedupKey; one public Seconds for all bounds, body `&str`/String, raw max_receive_count |
| BU | 3 | 5 | BU/src/types.rs:176 `Body` | BU/src/queue.rs:230 `deduplication: HashMap<String, Accepted>` | Typed ids incl. QueueId/ArrivalKey; single Seconds, raw u32/u64 for MaxReceiveCount and sequence |
| FA | 2 | 12 | FA/src/attributes.rs:10 `QueueKind` enum | FA/src/types.rs:13 `Seconds(pub u64)` (body String queue.rs:48) | Ids typed, but one pub Seconds, raw max count, String body and dedup key |
| RA | 2 | 12 | RA/src/queue.rs:49 `in_flight: Option<InFlight>` | RA/src/attributes.rs:21 stored `DeadLetter.queue: String` | Stored DLQ reference and reply view stay String; String body/dedup; pub Seconds |
| RO | 2 | 12 | RO/src/queue.rs:29 `Hold { Free, Delayed, InFlight { handle } }` | RO/src/queue.rs:84 `deduplication_id: Option<String>` | Good state enum and QueueName in DLQ; dedup String, body String, `ListQueues { prefix: String }` with "" as absent |
| SU | 2 | 12 | SU/src/queue.rs:27 `Availability` enum | SU/src/queue.rs:51 `sent_at: Seconds` | Timestamps and durations share one `Seconds(pub u64)`; reply `queue_names: Vec<String>`, dedup String |
| VA | 2 | 12 | VA/src/time.rs:5 `Instant` distinct from `Seconds` | VA/src/broker.rs:116 handler takes raw `Params`, returns `Map<String,Value>` | Id newtypes exist, but params parsed inside handlers, responses as JSON maps, `DeadLetter.queue: String`, two-timestamp state |
| LE | 1 | 17 | LE/src/names.rs:25 `QueueName` newtype | LE/src/attributes.rs:29 `visibility_timeout: u64` | Names typed, but all times, durations, counts, sequence and DLQ name are raw u64/String |
| NA | 1 | 17 | NA/src/command.rs:29 typed `Command` enum | NA/src/queue.rs:176 `handles: HashMap<u64, u64>` | No newtypes at all; ids, handles, group, dedup raw; responses built as `Members` maps |
| MA | 1 | 17 | MA/src/types.rs:47 `ReceiptHandle(u64)` | MA/src/handler.rs:226 delivery built with `json!` | Some id newtypes, but params parsed in handlers, every reply a JSON map, group/dedup String, two-timestamp state |
| QE | 0 | 20 | QE/src/error.rs:7 `ErrorCode` enum | QE/src/message_commands.rs:173 handle prefix parsed deep in handler | Only the error code is typed; u64 ids, `BTreeMap<String,Queue>`, params and replies as `Map<String,Value>` |
| WI | 0 | 20 | WI/src/error.rs:7 `ErrorCode` enum | WI/src/queue.rs:78 `handles: HashMap<u64, u64>` | Same shape as QE: command dispatched as a String inside the broker, no newtypes, JSON maps in and out |

## What separates the top from the bottom
- Bounded integers: the top crates give each range its own type (BO, KE, FO, LO, MO, XI, ZU, VU, ZE use `Bounded<MIN,MAX>` aliases or per-attribute newtypes, e.g. KE/src/scalar.rs:37); the middle uses one `Seconds(pub ..)` for every range (FA/src/types.rs:13, JU/src/time.rs:9, RO, RA, SU); the bottom stores `u64`/`u32` (LE/src/attributes.rs:29, WI/src/attributes.rs:28).
- Paired fields: MO/src/attributes.rs:103 and VU/src/request.rs:15 make the DLQ queue/count pair one enum, JU/src/queue.rs:136 pairs receive count with first receipt. MA, NA, QE, VA, WI keep `delayed_until`, `in_flight_until`/`invisible_until` and `handle: Option<..>` as separate fields (NA/src/queue.rs:94-98).
- Parse at the boundary: most crates build a typed `Command`/`Request` enum in one module. QE, WI, VA, MA parse `Params` inside each handler and hand back `Map<String, Value>` (WI/src/broker.rs:18, VA/src/broker.rs:20), so typed values never travel inward and the reply shape is not a type.
- Identifier newtypes: XI goes as far as `NonZeroU64` handles and QueueId/Arrival keys; NA, QE, WI have none, and LE stops at names (its `DeadLetter.queue` and reply queue names are String, LE/src/attributes.rs:23).
- Stored references: RA/src/attributes.rs:21, VA/src/config.rs:64, LE, NA, QE, WI store the DLQ target as String after it was validated, where the others store `QueueName`.

## Patterns seen in most crates
- Every crate has a closed `ErrorCode` enum and none compares error codes as strings; 20 of 21 use the same `ErrorCode + message: String` struct (BO alone types the cause).
- Nearly all keep the lookup queue name for non-create commands as `String`/`&str` (spec: any string is looked up); only VU wraps it, and its `QueueName` then has no invariant.
- The deduplication map is commonly keyed by bare `String` (BU, FA, RA, RO, SU, VA, VU, XI, NA, WI) even in otherwise well-typed crates; message body is String in about half.
- Message state is usually an enum (`Hold`/`Phase`/`Availability`) and "not set" uses `Option`; no crate uses numeric sentinels for unset attributes.
- Command names are matched as strings once at the boundary everywhere, which is fine; attribute names likewise.
