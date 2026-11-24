# p14g / types

## Method

Same procedure for all 12 crates, `src/` only (tests are not about the type model):

1. Extracted every `struct`, `enum` and `type` declaration with its fields from every file under `src/`
   (one awk pass, identical for each crate) and read all of them.
2. Ran the same greps in each crate: uses of `serde_json::Value` / `Map<String, ...>` per file (to see
   whether untyped JSON leaves the parsing boundary), string-literal comparisons, string dispatch on the
   command name (`"CreateQueue" =>` outside the parser), and sentinel patterns (`= 0`, `unwrap_or(0)`,
   `u64::MAX`).
3. Checked in each crate how a receipt handle text becomes a typed handle, and opened the few spots the
   greps flagged (state derivation, broker dispatch, error code rendering).

Questions asked of each crate: newtypes for ids/names/body/time; durations vs clock points; ranges as
types; FIFO-only data (group, sequence, content-based dedup) grouped in an enum or struct; message
hold state as an enum rather than parallel fields or 0 sentinels; a typed request and typed reply
rather than JSON maps past the boundary.

## Scores

| code | score | rank | best evidence | worst evidence | reason |
|---|---|---|---|---|---|
| SE | 4 | 1 | SE/src/types.rs:11 (`Bounded<MIN, MAX>` per range) | SE/src/fifo.rs:31 (`sequence_number: u64` beside a typed model) | Range types, `Kind::Fifo{..}`, `Phase` enum, `DedupKey`; only the sequence number is bare |
| XI | 4 | 1 | XI/src/types.rs:42 (bounded `VisibilityTimeout` etc.) | XI/src/command.rs:58 (`queue: String` in every command) | Range newtypes, typed `Error` with reasons, `Kind`, `Hold`, `DedupKey`; lookups by plain string |
| DO | 3 | 3 | DO/src/queue.rs:79 (`Kind::Fifo(Fifo)` owns FIFO-only state) | DO/src/queue.rs:57 (`accepted: HashMap<String, Accepted>`) | Body newtype, `Hold` enum, typed error reasons; generic `Seconds`, string dedup key and queue refs |
| CU | 3 | 3 | CU/src/queue.rs:25 (`Hold::InFlight { until, handle }`) | CU/src/queue.rs:131 (`deduplication: HashMap<String, Accepted>`) | `QueueKind::Fifo{cbd}`, `FifoPlacement`; body and dead-letter request kept as `String` |
| ZO | 3 | 3 | ZO/src/request.rs:21 (command name parsed to enum at boundary; `QueueId`) | ZO/src/queue.rs:149 (`sequence: u64`, also response.rs:46) | Instant vs Seconds split, `Kind`; sequence number and receive count bare integers |
| PA | 3 | 3 | PA/src/queue.rs:19 (`Hold` enum with handle in `InFlight`) | PA/src/broker.rs:61 (string dispatch, `Params` map handed to handlers) | Range newtypes and `Kind`; no typed request, standard queues keyed by `SequenceNumber` |
| CI | 3 | 3 | CI/src/types.rs:120 (`QueueRef` for lookups, `DeduplicationKey`) | CI/src/queue.rs:37 (`content_based_deduplication: bool` outside `QueueKind`) | Thorough newtypes; FIFO flag split from kind, state derived from `hidden_until` + `Option<handle>` |
| TA | 3 | 3 | TA/src/attributes.rs:12 (`QueueKind::Fifo{cbd}`) | TA/src/attributes.rs:21 (dead-letter target kept as `String`) | Time vs Seconds, `InFlight` struct; body and DLQ name are strings, one `Seconds` for all ranges |
| PE | 2 | 9 | PE/src/queue.rs:27 (`Availability` enum) | PE/src/broker.rs:43 (`now: Seconds`, clock and durations share a type) | Good hold enum and Body, but `fifo`/`cbd` bools (attributes.rs:28) and `Option<String>` dedup |
| HO | 1 | 10 | HO/src/command.rs:29 (typed `Command` enum) | HO/src/queue.rs:85 (Message all bare `u64`/`String`, `0` sentinels at :94-96) | Request typed, everything after it primitive; replies are `Map<String, Value>` (broker.rs:17) |
| HU | 0 | 11 | HU/src/queue.rs:138 (`State` enum) | HU/src/broker.rs:27 (`handles: Vec<(u64, u64)>`) | String dispatch with raw JSON map to handlers, bare ids, `0` sentinels (queue.rs:171), JSON replies |
| NO | 0 | 11 | NO/src/attributes.rs:27 (`Option<DeadLetter>`) | NO/src/request.rs:15 (`Request { command: String, members: Map }`) | Command dispatched as string (broker.rs:65), ids bare `u64`, sentinel `until` fields, JSON replies |

## What separates the top from the bottom

- Range types: SE (types.rs:11), XI (types.rs:42) and PA make `VisibilityTimeout`, `MaxMessages` etc.
  distinct types, so a delay cannot be passed where a timeout is expected; the middle uses one
  `Seconds`, and PE (broker.rs:43) even uses it for the clock.
- Message hold state: the top crates model it as one enum carrying the handle (CU queue.rs:25,
  SE queue.rs:55, PA queue.rs:19); HO, HU, NO keep `delayed_until` / `in_flight_until` with `0` meaning
  unset (HO queue.rs:94-96, NO queue.rs:164-165) and `handle: Option<u64>` beside them.
- FIFO-only data grouped: `Kind::Fifo { content_based_deduplication }` plus a `FifoPlacement`-style
  struct in CU, DO, SE, XI, TA, ZO; CI (queue.rs:37) and PE (attributes.rs:28) keep the flags apart,
  and HO/HU/NO carry `group_id` and `sequence_number` as two independent `Option`s.
- Boundary: CI, CU, DO, SE, XI, ZO, TA, PE parse a line into a typed request enum; PA, HU and NO match
  the command string in the broker and hand the JSON map to each handler (HU broker.rs:56,
  NO broker.rs:65); HO, HU, NO also build replies as `Map<String, Value>` instead of typed structs.
- Identifiers: the bottom three use `u64` for message ids, handles, arrival keys and sequence numbers
  interchangeably (HU broker.rs:27 `Vec<(u64, u64)>`, HO queue.rs:176 `HashMap<u64, u64>`).

## Patterns seen in most crates

- All twelve have a closed `ErrorCode` (or a typed error enum that renders one); none compares codes
  or attribute names as strings outside the parser.
- Nearly all keep `QueueName` as a validated newtype, but pass the lookup name inward as `String`/`&str`
  (only CI has a `QueueRef`); defensible, since lookup must accept any string.
- Receipt handles stay text in the request in every crate and are parsed at the handler against the
  issue counter; the stored handle is a newtype everywhere except HO, HU, NO.
- The deduplication key is a plain `String` map key in eight crates; only CI, SE and XI give it a type.
- `Option` is used for "not set" on `FirstReceivedAt` and the dead-letter configuration everywhere;
  sentinel `0` survives only in the three lowest crates.
