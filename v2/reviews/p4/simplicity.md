# p4 / simplicity

## Method

No `BASE` directory exists, so every crate was judged as a whole. For each of the 12 crates I read every
file under `src/` in full, up to the first `#[cfg(test)]` module, with comment-only and blank lines
removed so that all crates were read in the same form. Test modules and `tests/` were not read, since this
dimension is about production code. I also ran the same grep over every `src/` for `#[allow`, `dbg!`,
`eprintln!`, `todo!`, traits, generic functions, `dyn` and builders. None of them has an `#[allow]`, leftover
debugging, a trait or a builder, so the differences come from reading the code. In each crate I checked
the same points:

- whether CreateQueue and SetQueueAttributes share one merge helper;
- whether the standard and FIFO send paths copy each other;
- how ReceiveMessage is organized (direct loop, plan then apply, or re-lookups);
- wrapper and forwarding layers, and structs with only one user;
- branches for states that cannot occur;
- module count compared with problem size.

Line numbers are the real ones in the source files.

## Scores

| code | score | rank | best evidence | worst evidence | reason |
|---|---|---|---|---|---|
| XI | 4 | 1 | XI/src/queue.rs:115 | XI/src/queue.rs:227 | Small and direct: one `state()`, serde-derived responses, a lazy expiry index. The only lapse is that `send_standard` and `send_fifo` each build the `Message` (227 and 282). |
| FU | 3 | 2 | FU/src/request.rs:223 | FU/src/queue.rs:150 | Clean parsing and service code. The queue keeps six parallel indexes that every mutation, and `purge` at 358, must keep in sync. |
| KE | 3 | 2 | KE/src/queue.rs:72 | KE/src/request.rs:270 | One `apply` serves both create and set. Integers are turned back into strings and re-parsed by a custom `parse_integer` (212). |
| MU | 3 | 2 | MU/src/queue.rs:107 | MU/src/broker.rs:119 | Direct linear receive and one state decision. `create_queue` builds attributes as a deferred `Result` only to reorder the errors (144, 149). |
| SO | 3 | 2 | SO/src/attributes.rs:121 | SO/src/broker.rs:96 | No command enum, and the handlers parse directly. `require` and then `required_str` for the same name is copied into nine handlers. |
| TA | 3 | 2 | TA/src/attributes.rs:70 | TA/src/queue.rs:238 | Tidy modules. The FIFO decision is spread over `check_send` (205), `duplicate_of` (238) and `push` (265), and the dedup id is computed twice. |
| ZU | 3 | 2 | ZU/src/queue.rs:276 | ZU/src/queue.rs:203 | One message construction after the per-kind match. `Queue` forwards expire, counts, purge and receive to `MessageStore` unchanged. |
| NI | 2 | 8 | NI/src/queue.rs:73 | NI/src/broker.rs:348 | The receive loop looks the queue up again on every iteration and goes through a `Step` enum, with `if let` branches for impossible `None` (371, 379). It has 9 modules, and `replace_attributes` rebuilds the expiry index (147). |
| QE | 2 | 8 | QE/src/attributes.rs:163 | QE/src/broker.rs:259 | `apply_to` is reused well. Receive is examine, remove, then look the source up again three times with error paths that cannot happen. A global `deliveries` Vec repeats the per-message receipt check. |
| QI | 2 | 8 | QI/src/attributes.rs:161 | QI/src/reply.rs:95 | Getters on every struct (attributes.rs:178-200, queue.rs:66-93), and `CountsWire`/`AttributesWire` copy domain structs field for field for a single user. |
| WE | 2 | 8 | WE/src/store.rs:51 | WE/src/queue.rs:246 | The `Delivery` enum is a good single source. `Queue` is mostly a forwarding layer over `MessageStore`, and the standard and FIFO send branches each push their own message (171, 207). |
| PA | 1 | 12 | PA/src/request.rs:216 | PA/src/queue.rs:456 | `matches()` re-derives the defaults that `create()` (397) applies. It also adds a single-user `Store` wrapper (255), `Settings` split from `QueueAttributes`, `SendOutcome`, and plan/`Step` receive (670). The standard and FIFO sends duplicate the insert (549, 599). |

## What separates the top from the bottom

- **One merge helper or several.** The top crates use one helper: XI, KE (`KE/src/queue.rs:72`), SO
  (`SO/src/attributes.rs:121`), TA (`TA/src/attributes.rs:70`) and QE (`QE/src/attributes.rs:163`). PA
  checks equality field by field in `matches` (`PA/src/queue.rs:456`), separately from `create` (397),
  which is the same decision computed twice.
- **Forwarding layers.** WE (`WE/src/queue.rs:246`) and ZU (`ZU/src/queue.rs:203`) put a `MessageStore`
  under `Queue`, and many `Queue` methods only delegate to it. PA wraps a map plus an index in `Store`
  (`PA/src/queue.rs:255`). XI and MU keep the two collections directly on `Queue` and lose nothing.
- **How ReceiveMessage is written.** In the best crates it is one loop over arrival order, followed by
  removal of the dead-lettered messages (`MU/src/queue.rs:359`, `ZU/src/queue.rs:513`). The weaker ones
  split it into plan and apply with an intermediate enum and then look queues up again: NI
  (`NI/src/broker.rs:348`), QE (`QE/src/broker.rs:279-306`) and PA (`PA/src/queue.rs:644-694`).
- **Accessor and wire-struct ceremony.** QI adds getters to every struct and copies `Counts` into
  `CountsWire` (`QI/src/reply.rs:95`). XI and ZU serialize their domain structs directly with serde
  attributes (`XI/src/queue.rs:19`, `ZU/src/queue.rs:31`).
- **Duplicated send paths.** PA (`PA/src/queue.rs:549`/`599`), WE (`WE/src/queue.rs:171`/`207`) and XI
  (227/282) each build the message in two branches. ZU builds it once after the match
  (`ZU/src/queue.rs:276`).
- **Branches for impossible states.** NI's `deliver` silently returns on a missing key
  (`NI/src/queue.rs:256`). TA's `remove` has a `None => (None, true)` group arm (`TA/src/queue.rs:437`).
  QE's receive re-checks `queue_missing` (`QE/src/broker.rs:283`, 306). The better crates use an `expect`
  that names the invariant (`MU/src/queue.rs:474`).

## Patterns seen in most crates

- All 12 parse into a typed request or validate in place with a small `Params` helper. None has
  `#[allow]`, leftover debugging, a trait or a builder. Module layout is reasonable everywhere except NI
  (9 modules) and PA/QI (types split across many small wrappers).
- Nearly all of them copy the same CreateQueue/SetQueueAttributes prelude: the
  ContentBasedDeduplication check and the dead-letter target and type check, with only small
  differences. Only PA, QI and QE factor the target check into one helper.
- Most of them repeat the same four-line receipt-handle parse (`strip_prefix("r-")`, a leading-zero
  check, digits only), which is fine as each crate has only one copy. Three crates (KE, MU, ZU) decode
  integers by re-reading the number text, a roundabout path where `as_u64` would do.
- Counters (`Counter`, `Ids`, `MessageIds`/`ReceiptHandles`) are small and justified. The heavier
  index sets in FU, KE, QI and TA are defensible for the 100000-message performance requirement, but
  they cost every mutation path some bookkeeping.
