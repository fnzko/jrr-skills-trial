# p6g / overall

## Method
No `BASE` directory, so each crate was judged as a whole. For all 18 crates I used the same steps:
(1) `wc -l` of every `src/` and `tests/` file to see the layout; (2) the type and enum declarations
across `src/` (grep for `struct`/`enum`); (3) `lib.rs` and `error.rs` read in full; (4) the queue's
`receive` function (plus its `plan_receive` helper where there is one), about 60 lines; (5) the
`create_queue` handler in the dispatch layer, about 40 lines; (6) `delete_message`/`change_visibility`
and the `Message` struct; (7) the same greps in every crate for `json!`, `Map<String, Value>`
responses, `as` casts, `.expect("required")`, `unwrap_or_default`, `#[allow]`, comment counts, and
assertion and test counts. I read the queue struct of the crates that keep extra indexes (DE, TU, JU,
NE) to judge what has to be learned. I did not read every test file. Scores weigh readability, how
safely a change can be made, and how much has to be learned before the code can be trusted.

## Scores
| code | score | rank | best evidence | worst evidence | reason |
|---|---|---|---|---|---|
| CA | 4 | 1 | CA/src/error.rs:52 (one variant per code, typed sub-reasons) | CA/src/queue.rs:378 (`try_from` + expect on a bounded value) | Typed parse -> command -> service -> reply pipeline; errors carry typed causes; receive is one readable loop |
| CE | 4 | 1 | CE/src/queue.rs:385 (receive split into plan and apply) | CE/src/queue.rs:345 (expects tie plan to apply) | `Bounded<MIN,MAX>` (CE/src/scalar.rs:7) and text newtypes; small focused modules; clean `serve` |
| NI | 4 | 1 | NI/src/queue.rs:339 (plan_receive documented why plan-then-apply is sound) | NI/src/request.rs:244 (`unwrap_or_default` on Attributes) | Typed throughout, short handlers, Availability enum, compact tests |
| JU | 3 | 4 | JU/src/deliveries.rs:14 (handle log keyed by QueueId detects re-created queues) | JU/src/queue.rs:262 (2.4k src lines, const-generic types, `oldest_sent_at` bound to learn) | Very safe types, but the largest crate here and slow to learn |
| TA | 3 | 4 | TA/src/queue.rs:297 (Hold enum, typed handles in receive) | TA/src/attributes.rs:81 (`matches_request` sits apart from `from_request`, so "after defaults" is defined twice) | Clean typed layers; attribute equality is a second code path |
| RU | 3 | 4 | RU/src/queue.rs:275 (Hold::InFlight carries handle and deadline together) | RU/src/service.rs:122 (queue name stays a bare `String`, checked with `ends_with`) | Tidy and small; names untyped past the parser |
| VE | 3 | 4 | VE/src/queue.rs:446 (Step enum makes the receive decision explicit) | VE/src/queue.rs:437 (`max_messages as usize`) | Typed ids/names, Availability enum, good tests; minor casts |
| NO | 3 | 4 | NO/src/queue.rs:354 (plan_receive, the dead-letter case unblocks its group) | NO/src/queue.rs:80 (`body: String`, time as `Duration` reused for points in time) | Readable plan/apply, small surface; a few loose types |
| QA | 3 | 4 | QA/src/broker.rs:54 (single handle_line with an invariant-named expect) | QA/src/queue.rs:173 (`group: Option<String>`, `sequence: Option<u64>` set independently) | Compact and clear; FIFO fields not tied together |
| LA | 3 | 4 | LA/src/queue.rs:235 (one-pass receive, comments state why) | LA/src/queue.rs:30 (`visible_at` means delayed or in-flight depending on history) | Small and readable; one overloaded state field |
| KO | 2 | 11 | KO/src/queue.rs:324 (dead-letter does not hold back its group, with the reason given) | KO/src/queue.rs:65 (all-`pub` Message, bare `u64` ids, `String` group) | Clear flow but nothing stops the broker from corrupting queue state |
| HU | 2 | 11 | HU/src/queue.rs:255 (examine-then-apply receive) | HU/src/broker.rs:12 (responses built as `Map<String, Value>` via `json!`) | Request enum is all `String`/`u32`; output shape not checked by types |
| DE | 2 | 11 | DE/src/error.rs:7 (closed ErrorCode with as_str) | DE/src/queue.rs:115 (seven parallel indexes, stale-heap entries, to keep in sync) | Strong tests, but queue invariants are costly to learn and easy to break |
| TU | 2 | 11 | TU/src/types.rs:13 (Bounded newtype for ranges) | TU/src/queue.rs:171 (visible/holds/expiry/groups/receipts sets maintained by hand) | Typed, but same index-sync burden as DE for a spec that needs a linear scan only |
| NE | 2 | 11 | NE/src/values.rs:14 (Ranged const-generic values) | NE/src/queue.rs:194 (Received returns Vec indices "valid until the queue is next changed") | Typed values, but fragile index contract over `Vec<Message>`; almost all tests are unit tests in broker/tests.rs |
| SO | 2 | 11 | SO/src/queue.rs:369 (`remains` flag states the group rule plainly) | SO/src/broker.rs:172 (attributes assembled by hand with `json!` into a map) | Command enum exists, but bare `u64` ids and untyped responses |
| JI | 1 | 17 | JI/src/queue.rs:391 (decide-then-apply receive) | JI/src/message_commands.rs:19 (params parsed inside handlers, `.expect("required")`, `as u32` at :68) | No typed command layer; validation, domain and JSON mixed in each handler |
| LI | 1 | 17 | LI/src/queue.rs:194 (plan_receive is clear) | LI/src/broker/message_commands.rs:57 (same handler-side parsing + `.expect("required")`, `json!` at :39) | Same shape as JI: stringly params and responses, handlers do everything |

## What separates the top from the bottom
- A typed boundary. CA/CE/NI parse each line into a `Command` of newtypes (CA/src/command.rs:52,
  CE/src/command.rs:59) and render a typed `Output`. JI/LI pass a raw `Params` map into every handler
  and assume earlier checks passed, `params.string(..)?.expect("required")` (JI/src/message_commands.rs:19,
  LI/src/broker/queue_commands.rs:21), so a reordering silently becomes a panic.
- Typed responses. Crates that `Serialize` an `Output` enum (CE/src/output.rs:35, RU/src/response.rs:10)
  let the compiler hold the "exactly these members" rule. HU/SO/JI/LI build `Map<String, Value>` with
  `json!` (HU/src/broker.rs:204, SO/src/broker.rs:172), so a misspelled member name compiles.
- State modeled as an enum vs. parallel fields. NE/RU/TA/VE/NI have a `Hold`/`Availability` enum with the
  handle inside; KO/LI/SO keep `delayed_until`, `in_flight_until`, `handle: Option<u64>` side by side
  (SO/src/queue.rs:85), which allows combinations that cannot occur.
- Proportionate data structures. DE and TU keep 5 to 7 indexes in sync (DE/src/queue.rs:115,
  TU/src/queue.rs:171) where a single `BTreeMap` scan (CA, CE, NI) passes the spec; every future change
  to them has to re-establish every invariant.
- Error types. CA's `Error` enum with typed causes (CA/src/error.rs:52) is the easiest to match and
  extend. Most others use `ErrorCode + String message`, which is adequate. None return `String` errors.

## Patterns seen in most crates
- Nearly all follow the spec's check order closely in `create_queue` (fifo suffix -> CBD -> DLQ pair ->
  exists -> DLQ lookup -> type -> limit), with near-identical structure, so readers learn one and can read all.
- Nearly all split receive into "decide which messages, then mutate" and call `expect` only for
  invariants they name. Raw panics on input are rare.
- Doc comments are used sparingly and mostly state rules or invariants; no crate has `#[allow]` or
  leftover debugging.
- Integration tests through the stdin/stdout loop with exact JSON assertions are the norm. NE is the
  exception: it tests mostly at unit level.
