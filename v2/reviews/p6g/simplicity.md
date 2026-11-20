# p6g / simplicity

## Method

No `BASE` directory exists, so every crate was judged whole. The same steps were applied to all 18 crates:

1. The same greps over `src/` and `tests/`: `#[allow`/`#![allow`, `dbg!`/`println!`/`eprintln!` outside `main.rs`, `todo!`/`unimplemented!`/`unreachable!`/`panic!`, `trait`, generic `impl<`/`fn x<`, module count and non-test `src` line count.
2. A script counting single-use and unreferenced `fn`s per crate. Its output did not separate the crates, so it was only a cross-check.
3. A full read of the command-handler layer (the `broker`/`service` file, or the `*_commands` files) and of `queue.rs`, up to `#[cfg(test)]`, with blank and comment lines removed. In these files I looked for blocks copied across handlers, decisions made in two places, parallel types for one concept, indirect loops, caches or indexes the spec does not need, and impossible branches.

Parsing and type modules (`request`, `types`, `params`, `attributes`) were only covered by step 1, so they had the same depth of review in every crate. For JI I happened to see the whole crate, but scored it from the same files as the others.

## Scores

| code | score | rank | best evidence | worst evidence | reason |
|---|---|---|---|---|---|
| LA | 4 | 1 | LA/src/queue.rs:38 (state derived from one `visible_at` plus handle, no second enum) | LA/src/service.rs:192 (CBD and partial-DLQ checks repeated from create_queue) | Flattest model and direct single-pass receive (queue.rs:254); only the spec-shaped create/set overlap repeats |
| CE | 3 | 2 | CE/src/service.rs:171 (one `dead_letter_config` serves create and set) | CE/src/queue.rs:83 (`Availability` plus `State` for one concept, then a plan/`Step` two-pass receive at :385) | Clean handlers; the queue model is doubled |
| HU | 3 | 2 | HU/src/queue.rs:255 (plain examine loop) | HU/src/broker.rs:158 (three lookups of the same queue, then `expect`) | Direct code; small redundancies, plus a `members()` helper with `unreachable!` at :431 |
| KO | 3 | 2 | KO/src/service.rs:46 (`overlay` shared by create and set) | KO/src/service.rs:179 (DLQ validation block copied from :137) | Simple data model; copied blocks between handlers |
| NI | 3 | 2 | NI/src/broker.rs:147 (`dead_letter_target` shared) | NI/src/queue.rs:166 (`swept_at` memo with `refresh` vs `expire`; broker.rs:292 inlines a copy of `queue()`) | Tidy, but has an extra cache field and a duplicated lookup |
| NO | 3 | 2 | NO/src/service.rs:157 (`dead_letter_for` shared) | NO/src/queue.rs:319 (`unreachable!` branch for a state that cannot occur) | Mostly direct; plan/`Step` and `Phase`/`State` add a little indirection |
| QA | 3 | 2 | QA/src/queue.rs:367 (single-pass receive) | QA/src/queue.rs:101 (`matches` recomputes the defaults `Settings::new` at :76 already applies) | Small and direct; one decision is made in two places |
| RU | 3 | 2 | RU/src/service.rs:50 (retention applied in one place, on Tick) | RU/src/service.rs:237 (inline copy of `queue_mut` lookup, again at :305) | Lean; a few duplicated lookups and copied Delete/Change arms |
| TA | 3 | 2 | TA/src/broker.rs:130 (`resolve_dead_letter` with `owner` removes the create/set copy) | TA/src/broker.rs:218 (second lookup of the queue after :201) | Good factoring; minor double lookups and `Hold`/`State` pair |
| VE | 3 | 2 | VE/src/broker.rs:63 (expiry on Tick, one place) | VE/src/queue.rs:446 (three-way `Step` with `Skip` computed then re-matched in the loop) | Direct overall; the receive loop is less direct than it needs to be |
| CA | 2 | 11 | CA/src/service.rs:272 (`live_queue` lookup+expire helper) | CA/src/service.rs:164 (DLQ resolution copied from :134; `self.queue(name)?` at :197 called only for its side effect) | `Phase`+`Hold`, `kind_of`/`existing_name` double lookups, CBD rule in `Kind::from_flags` and in `set_attributes` |
| DE | 2 | 11 | DE/src/broker.rs:329 (`with_delivery` closure shares Delete/Change) | DE/src/queue.rs:128 (ready set, hidden `BinaryHeap`, group index: six indexes kept in sync) | Clean handlers over a queue built for scale the spec does not ask for |
| JI | 2 | 11 | JI/src/queue.rs:377 (single-pass receive) | JI/src/message_commands.rs:19 (`require` then `.expect("required")` in every handler) | Required check made twice per parameter, plus a double `queue_mut`/`expire` in queue_commands.rs:113 |
| LI | 2 | 11 | LI/src/broker/message_commands.rs:204 (`in_flight_message` shared) | LI/src/queue.rs:255 (dedup pruning threshold field nobody needs) | Same require/expect double decision as JI (message_commands.rs:56-58), plus plan/`Step` |
| NE | 2 | 11 | NE/src/broker.rs:113 (expiry on Tick, one place) | NE/src/broker.rs:354 (queue looked up again to turn returned indices back into messages) | Largest src (2820 lines); `QueueType` mirrors `Kind` (queue.rs:228), `Requested`/`Update`/`DeadLetterRequest`, getter boilerplate, `Sent.accepted` flag |
| SO | 2 | 11 | SO/src/broker.rs:283 (`find_in_flight` shared) | SO/src/queue.rs:337 (if/else-if yielding bools with side effects inside the loop) | `swept_at` cache needing manual `invalidate_sweep` (queue.rs:221); partial DLQ dropped by `dead_letter_of` while validated separately |
| TU | 2 | 11 | TU/src/service.rs:141 (`resolve_dead_letter` shared) | TU/src/queue.rs:175 (visible/holds/expiry/receipts/groups indexes, `sync` called twice in `set_attributes`) | Speculative multi-index queue, a `FromValue` trait for parsing, and `#[allow(dead_code)]` in tests |
| JU | 1 | 18 | JU/src/broker.rs:191 (`resolve_dead_letter` shared) | JU/src/queue.rs:107 (`Kind` duplicates `QueueType` at :27) | Parallel types throughout: `QueueSpec`/`Settings`/`Update`/`Attributes`, `DeadLetterRequest`/`DeadLetter`, `Availability`/`State`, plan/`Action`, `Seconds<300,300>` |

## What separates the top from the bottom

- **One representation per concept.** LA keeps a message's state in `visible_at` plus an optional handle (LA/src/queue.rs:38). JU and NE keep a public `QueueType` alongside an internal `Kind` and convert between them (JU/src/queue.rs:27 and :107, NE/src/queue.rs:228). JU adds three more attribute structs on top.
- **Plain data structures for the job.** The spec only asks that sends to a full queue be fast. DE (DE/src/queue.rs:124-131) and TU (TU/src/queue.rs:175-179) still maintain 5 or 6 synchronized indexes, while HU, LA and QA do the same work with one `BTreeMap` plus an age index.
- **Shared helpers for what create and set share.** TA (TA/src/broker.rs:130) and CE (CE/src/service.rs:171) resolve the dead-letter target once. KO (KO/src/service.rs:137 vs :179), CA (CA/src/service.rs:134 vs :164) and others copy the block.
- **No caches that need manual upkeep.** SO's `swept_at` needs `invalidate_sweep` (SO/src/queue.rs:221), NI's `swept_at` adds a `refresh` vs `expire` split (NI/src/queue.rs:166), and LI keeps a pruning threshold (LI/src/queue.rs:255). RU, NE and VE avoid all of this by expiring on `Tick`, the only thing that moves the clock (RU/src/service.rs:50).
- **The required check made once.** JI and LI check that a parameter is present, then `.expect("required")` it again in every handler (JI/src/message_commands.rs:19). Crates that parse into typed commands make that decision once.

## Patterns seen in most crates

- Nearly every crate repeats the `ContentBasedDeduplication requires FIFO` check and the dead-letter type check in both CreateQueue and SetQueueAttributes. Most also have near-identical DeleteMessage and ChangeMessageVisibility match arms. The spec's two rule lists invite this, and few crates factor it out.
- Most crates pair a stored hold enum (`Hold`, `Availability`, `Phase`) with a derived `State` enum. About half split receive into a plan pass and an apply pass with a `Step`/`Action` enum. Both work, but they cost a type each without being needed.
- None of the 18 have leftover debugging, `todo!`, or `#[allow]` in `src/`. Seven (DE, HU, JI, KO, LI, SO, TU) put `#![allow(dead_code)]` on a shared `tests/common` module.
- Module layouts are reasonable everywhere, at 7 to 11 files. NE and CE are at the heavy end with separate `time`/`values`/`scalar`/`text` modules.
