# p4 / overall

## Method
No `BASE` directory exists, so each crate was judged whole. The same files and checks were used for every crate:
- Read in full: `lib.rs`, `main.rs`, and the state/dispatch module (`service.rs`/`broker.rs`), plus the protocol/response module where there is one.
- Read the `ReceiveMessage` core (the `receive`/`examine`/`plan_receive` function and what it calls) in `queue.rs` or `store.rs`, since this is the hardest part of the spec and the most likely place a future change goes wrong.
- Ran the same greps across the crate: `unwrap`/`expect` outside `#[cfg(test)]`, `as` casts, `#[allow]`, `Value` counts, comment counts, test counts, and `assert_eq!` vs `assert!`.
- Skimmed the head of one integration test file per crate.

Request parsers (`request.rs`/`command.rs`) were not read line by line in any crate. I did not run cargo.

## Scores
| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| TA | 4 | 1 | TA/src/broker.rs:14 | TA/src/broker.rs:291 | Smallest src (1490 lines). Typed `Reply`, direct handlers, a well-commented ready-index receive, 102 tests. Names and handles stay bare `String`/`u64`. |
| ZU | 4 | 1 | ZU/src/response.rs:12 | ZU/src/queue.rs:320 | Typed serde `Output`, newtype ids/time, very short dispatch. One `panic!` arm for a type mismatch that cannot happen. |
| WE | 3 | 3 | WE/src/lib.rs:67 | WE/src/broker.rs:278 | Typed error enum and `Kind::Fifo(FifoState)`, serde output end to end. `receive` takes the queue out of the map and puts it back to dodge the borrow checker. |
| MU | 3 | 3 | MU/src/types.rs:41 | MU/src/broker.rs:119 | Validated `QueueName`, `Hold` and `QueueKind` enums, typed errors. `create_queue` threads a transposed `Result` past the existing-queue rule, which is hard to follow. |
| FU | 3 | 3 | FU/src/service.rs:18 | FU/src/service.rs:145 | Clear typed `Response` and readable rule order. The eligibility index takes study, ids are bare `u64`, and defaults are rebuilt per field. |
| XI | 3 | 3 | XI/src/broker.rs:29 | XI/src/broker.rs:184 | Clean select-then-apply receive and typed `Response`. Expiry is lazy, spread over `lookup`, and a re-lookup needs `expect`. |
| QI | 3 | 3 | QI/src/time.rs:24 | QI/src/time.rs:8 | Strong newtypes and a typed reply. Getter-heavy config adds surface, and it invents a clock cap that the spec leaves unspecified. |
| KE | 2 | 8 | KE/src/queue.rs:365 | KE/src/service.rs:309 | Good receive walk with reasons in comments. Handlers return `Map<String, Value>`, so JSON leaks into the domain, with an `unreachable!` to recover the object. |
| PA | 2 | 8 | PA/src/broker.rs:16 | PA/src/broker.rs:230 | Rich types and a plan/apply split. A receipt-location map grows forever, `Option<ReceiptHandle>` encodes "unparseable", and the `range_mut` lookup trick adds indirection. |
| QE | 2 | 8 | QE/src/queue.rs:298 | QE/src/broker.rs:279 | Readable `examine`. The broker builds JSON maps, `receive` looks the queue up three times and drops misses with `filter_map`, and the delivery log grows forever. |
| NI | 2 | 8 | NI/src/broker.rs:438 | NI/src/broker.rs:370 | The `Step` enum names the receive decision. The broker hand-builds JSON, re-fetches the queue each loop turn, and `if let Some` hides impossible misses. |
| SO | 1 | 12 | SO/src/queue.rs:216 | SO/src/broker.rs:76 | No typed request. The broker dispatches on command strings and parses params inside each handler, with `as` casts (SO/src/broker.rs:344). Its large test suite cannot make up for that boundary. |

## What separates the top from the bottom
- **A typed boundary in both directions.** TA, ZU, WE, MU and FU parse into a `Request`/`Command` enum and return a typed `Reply`/`Output` that is rendered in one place (ZU/src/response.rs:48, WE/src/lib.rs:42). SO keeps `serde_json::Map` all the way into the handlers (SO/src/broker.rs:74-92). KE, QE and NI type the input but build JSON inside the broker (KE/src/service.rs:45, QE/src/broker.rs:19, NI/src/broker.rs:22). A change to a response shape then touches the business logic.
- **Receive written as one direct pass.** TA (TA/src/queue.rs:326), QI and FU walk an index with a cursor and mutate in place, with one comment saying why the cursor exists. QE re-borrows the map three times across plan, move and deliver (QE/src/broker.rs:266-306). NI re-looks up the source queue on every iteration (NI/src/broker.rs:349).
- **No unbounded bookkeeping.** PA (PA/src/broker.rs:32), QE (QE/src/broker.rs:35) and SO (SO/src/broker.rs:45) store one record per delivery for the life of the process to resolve receipts. The others resolve a handle through a per-queue map that is pruned on redelivery and removal.
- **Amount to learn before a change is safe.** TA and ZU read top to bottom in one sitting. PA, QI and WE add lifetime-carrying outputs, `range_mut` lookups or remove-and-reinsert tricks (PA/src/broker.rs:315, WE/src/broker.rs:278) that a maintainer has to understand before touching receive.
- **Impossible states.** The better crates use `expect` that names an invariant. ZU still has a `panic!` match arm (ZU/src/queue.rs:320). NI silently ignores `None` from a lookup that cannot fail (NI/src/broker.rs:371, 379).

## Patterns seen in most crates
- Every crate keeps the check order of SPEC section 2 visible in one dispatch module, and every crate checks all rules before the first mutation. Rule-order changes are low risk everywhere.
- Nearly all of them use a `BTreeMap` keyed by name for `ListQueues` byte order and a shared byte-level line loop with flush per response. That code is near-identical and correct-looking across the set.
- Most use lazy expiry on lookup (`live_queue`/`find`/`lookup` that call `expire`). This spreads a time-dependent invariant over every access path. FU, KE, QI and SO also expire eagerly on `Tick`.
- Queue names and receipt or message numbers stay bare `String`/`u64` in about half the crates (FU, KE, NI, SO, TA). The others use newtypes, but the handlers are just as readable either way.
- `expect` messages almost always state the invariant. No crate has `#[allow]` or `unwrap` in non-test code paths beyond a handful.
