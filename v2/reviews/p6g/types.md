# p6g / types

## Method

There is no `BASE` directory, so I judged each crate as a whole. I used the same four passes on the `src/` of all 18 crates. I did not read the tests because this dimension is about production types.

1. A script dumped every `struct`, `enum` and `type` definition with its fields, which gives the full shape of the data model.
2. One grep listed every function that takes a raw `Value` or `Map<String, Value>`, to see how far untyped JSON reaches.
3. A second grep listed string matches on closed sets (command names, attribute names, `.fifo`, `r-`/`m-` prefixes). I checked whether each one sits at the parse boundary or in domain code.
4. A third grep counted `json!(...)` outside tests and inline `format!("m-..")`/`format!("r-..")`, to find responses built by hand as JSON maps.

I opened source only to confirm what a hit meant. That meant handler signatures in JI, LI, SO and HU, and the `Message` state fields in CE, KO, DE, NE, HU and JI.

Not counted against anyone: every crate keeps the lookup `QueueName` of non-create commands and the `ReceiptHandle` input as `String`. The spec requires lookup by exact string and "never issued" detection, so all crates are equal on this point.

## Scores

| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| CA | 4 | 1 | CA/src/queue.rs:21 (`Kind::Fifo { content_based_deduplication }`) | CA/src/command.rs:18 (`dead_letter_queue: Option<String>`) | Typed values throughout (ranged-seconds macro, Time, Body, GroupId, DedupKey); error sub-enums; FIFO data paired. |
| CE | 4 | 1 | CE/src/scalar.rs:7 (`Bounded<MIN, MAX>`) | CE/src/queue.rs:106 (`handle` kept apart from `Availability::InFlight`) | Every scalar and text value has a type; one place where fields that belong together are split. |
| JU | 4 | 1 | JU/src/queue.rs:118 (`InFlight { until, handle }`), JU/src/queue.rs:109 (`Kind::Fifo(FifoState)`) | JU/src/queue.rs:95 (`HashMap<String, DeduplicationEntry>`) | Richest model: FIFO-only state inside the enum variant, `SendOutcome`, `NonZeroU64` handle, `QueueId`/`Arrival`. |
| NE | 3 | 4 | NE/src/queue.rs:166 (`HashMap<DeduplicationId, Accepted>`), NE/src/request.rs:21 (`CommandName` enum) | NE/src/queue.rs:191 (`accepted: bool` flag instead of an outcome enum) | Strong newtypes; lapses are the bool flag, `Hold::None`, `delivered: Vec<usize>` indices and an untyped body. |
| NI | 3 | 4 | NI/src/queue.rs:34 (`InFlight { until, handle }`) | NI/src/queue.rs:109 (`deduplication_id: Option<String>`) | Good ids, `Admission` enum, `FifoInfo`; one `Seconds` type for every range; dedup id and max-receive count bare. |
| NO | 3 | 4 | NO/src/queue.rs:70 (`Phase::Delivered { until, receipt }`) | NO/src/request.rs:128 (`deduplication_id: Option<String>`) | Clean state enum and `QueueKind`; body and dedup id bare; clock instants typed as `Duration`. |
| TA | 3 | 4 | TA/src/attributes.rs:23 (`DeadLetter { queue: QueueName, .. }`) | TA/src/queue.rs:131 (`HashMap<String, Accepted>`) | `QueueKind`, `Hold`, `FifoPlacement`, `Time`/`Seconds`; body, dedup key and max-receive count bare. |
| TU | 3 | 4 | TU/src/types.rs:13 (`Bounded<MIN, MAX>`) | TU/src/queue.rs:119 (`sent_at: u64`; the clock is a bare `u64` everywhere) | Ranged attributes and id newtypes; time and body left as primitives. |
| DE | 2 | 9 | DE/src/domain.rs:40 (`ReceiptHandle(u64)`) | DE/src/attributes.rs:23 (`fifo: bool` plus a separate `content_based_deduplication: bool`) | Id newtypes exist, but the core stores durations as `u64`, the body as `String`, the queue map by `String` and the DLQ name as `String`. |
| LA | 2 | 9 | LA/src/ids.rs:12 (`QueueName`) | LA/src/queue.rs:72 (`type QueueId = u64`) | Ids typed; attributes are raw `u32`, group is `String`, `visible_at` and `handle` are separate fields. |
| QA | 2 | 9 | QA/src/queue.rs:153 (`Delivery { handle, until }`) | QA/src/queue.rs:173 (`group: Option<String>` and `sequence: Option<u64>` as separate options) | Id and name newtypes; raw attribute ints; FIFO data split; the output view splits the DLQ pair. |
| RU | 2 | 9 | RU/src/queue.rs:23 (`Hold` enum) | RU/src/request.rs:12 (CreateQueue `name: String`, checked later with `ends_with(".fifo")` at service.rs:124) | Good enums (`QueueKind`, `Hold`, `FifoSlot`), but the queue name is never typed and attribute values are bare. |
| VE | 2 | 9 | VE/src/queue.rs:104 (`InFlight { handle, until }`) | VE/src/queue.rs:27 (DLQ `queue: String`) | State enum and `FifoPosition`, but attributes, time, sequence and dedup id are primitives; `fifo` and CBD are two bools. |
| HU | 1 | 14 | HU/src/ids.rs:15 (`ReceiptHandle`) | HU/src/broker.rs:204 (responses built as `json!` maps, `Members = Map<String, Value>`) | Has a `Request` enum, but output data with a known shape is untyped JSON; names, groups and sequence numbers are bare. |
| KO | 1 | 14 | KO/src/queue.rs:58 (`Delivery { handle, invisible_until }`) | KO/src/queue.rs:175 (`oldest_sent_at: u64`, `u64::MAX` sentinel) | Message id and handle are `u64`, ids are formatted to `String` in replies, and handle locations are tuples. |
| SO | 1 | 14 | SO/src/queue.rs:131 (`SendOutcome` enum) | SO/src/broker.rs:321 (`json!(format!("m-{}", ..))` output) | No newtypes at all; time is split into `delayed_until`/`in_flight_until`/`handle`; responses are JSON maps. |
| JI | 0 | 17 | JI/src/queue.rs:138 (`State` enum) | JI/src/message_commands.rs:55 (handler builds JSON with `format!("m-..")`) | No command type: handlers parse raw params inline and return JSON maps; ids are `u64`; handles are `Vec<(u64, u64)>`. |
| LI | 0 | 17 | LI/src/queue.rs:52 (`Step` enum) | LI/src/request.rs:16 (`command: String` + raw `members` map carried into broker.rs:66) | Untyped from input to output: the string command, a raw map, `u64` ids and handles, JSON-map replies. |

## What separates the top from the bottom

- **One typed command at the boundary versus raw JSON in the handlers.** CA, CE, JU, NE and others parse into a `Command` enum with typed fields. JI (JI/src/message_commands.rs:19) and LI (LI/src/broker.rs:63) give handlers a `Params` view over the raw map, so validation and domain logic are mixed together.
- **Typed or untyped responses.** The top 13 serialize response structs and enums. HU, SO, JI and LI build `Map<String, Value>` with `json!` and format ids by hand (SO/src/broker.rs:325, JI/src/message_commands.rs:182).
- **Range-checked attribute types.** CE, JU, NE and TU use a const-generic `Bounded`/`Ranged`/`Seconds<MIN, MAX>`, and CA uses a macro. The range is enforced once and travels with the value. DE, LA, QA, RU, VE, KO, HU and SO carry `u32`/`u64` and re-check ranges by hand.
- **Fields that only make sense together.** The best crates put the handle inside the in-flight state (JU/src/queue.rs:118, NI/src/queue.rs:34, NO/src/queue.rs:70, VE/src/queue.rs:104). The weak ones keep parallel `*_until: u64` timestamps next to an `Option` handle (SO/src/queue.rs:94-98, JI/src/queue.rs:154-157). FIFO is an enum carrying CBD (CA, CE, JU, NE, NO, RU, TA, TU) in some crates and `fifo: bool` plus `content_based_deduplication: bool` (DE, HU, JI, LA, LI, QA, SO, VE) in others.
- **Sentinels versus `Option`.** KO stores `u64::MAX` for "no messages" (KO/src/queue.rs:175). JU and LI use `Option` for the same field. NE has a `Hold::None` variant and an `accepted: bool` flag where an outcome enum belongs; JU (`SendOutcome`) and NI (`Admission`) use an enum.

## Patterns seen in most crates

- Good: all 18 parse the command name and attribute names with a `match` only at the boundary. None compares those strings again in domain code. 16 of 18 have a typed `Command`/`Request` enum. All 18 store the dead-letter configuration as one `Option<DeadLetter>` struct rather than two options. KO, QA and RU split it back into two options only in their output view.
- Good: nearly all model message state as a closed `State` enum for the counts and turn `r-<n>` into a typed handle in one function.
- Weak: the deduplication key is a bare `String` map key in 14 crates (only CA, CE and NE type it, and JI uses `Rc<str>`). The message body is a bare `String` in 14 (only CA, CE, JU and NI have a `Body` type).
- Weak: the request-side `DeadLetterQueue` stays `Option<String>` everywhere. That is defensible, since the spec allows any string there. But in DE, HU, JI, KO, LA, LI, RU, SO and VE it is still a `String` after validation, inside the stored queue config.
