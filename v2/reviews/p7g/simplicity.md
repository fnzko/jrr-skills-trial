# p7g / simplicity
## Method
Every crate got the same two passes:
- A mechanical pass over `src/`:
  - non-blank line counts
  - counts of `#[allow]`/`#[expect]`, `dbg!`, print macros outside `main.rs`, `unreachable!`/`todo!`, traits, generic impls, `macro_rules!` and builders
  - a per-function count of call sites in `src/` and `tests/`, to find unused or single-use helpers
- A full read of every `src/` file, leaving out `#[cfg(test)]` modules, `tests/` and LO's `src/broker/tests.rs`, with blank and comment-only lines stripped. Each crate was checked against the same list:
  - helpers, types, macros or fields with a single user
  - decisions made twice: the CreateQueue vs SetQueueAttributes checks, applying defaults, classifying message state, retention, receipt-handle validity
  - blocks copied across handlers, such as checking a parameter as required and then fetching it again
  - plan-then-apply receive loops
  - index maps that repeat data the message already holds
  - branches for states that cannot occur, and memo fields
  - module count against crate size

The reading was split into three batches of seven crates, all run with the same checklist. The scores were then calibrated across all 21: the same pattern costs the same everywhere, for example the double required-parameter check in WI, QE, VA, VU, MA and ZU. No crate has `#[allow]`, `dbg!`, leftover prints or real dead code. The only function the call-site count flagged as unused (`add`) is a std `Add` impl.

## Scores
| code | score | rank | best evidence | worst evidence | reason |
|---|---|---|---|---|---|
| SU | 4 | 1 | SU/src/queue.rs:59 one `Message::state` serves counts, receive and in_flight | SU/src/request.rs:151 free `seconds()` repeats `Fields::optional_seconds` (:76) | `require` returns the value (request.rs:68). Only small lapses: defaults applied in `create` and `matches` (attributes.rs:61/76), a dead `if let` (queue.rs:441) |
| FA | 3 | 2 | FA/src/broker.rs:130 `resolve_dead_letter` shared by create and set | FA/src/broker.rs:165 CBD and dead-letter pair checks copied at :208-216 | one direct receive pass (queue.rs:309-352); a macro for only 2 types (types.rs:52) |
| BU | 3 | 3 | BU/src/broker.rs:100 `dead_letter_target` holds the checks once | BU/src/queue.rs:396 receive builds a `Verdict` plan and then replays it | a second `Command` enum mirrors `Request` just to check the name (request.rs:21-51) |
| LO | 3 | 4 | LO/src/queue.rs:106 `Hold::state()` is the only state decision | LO/src/broker.rs:288 CBD check and `DeadLetter` build repeat :224-264 | `take_next` copied 3 times (values.rs:160,181,213); 4 overlapping attribute structs (queue.rs:35-65) |
| RA | 3 | 5 | RA/src/service.rs:152 `check_dead_letter_queue` shared; writes go through `from_patch`/`apply` | RA/src/queue.rs:276 plan-then-apply receive with a `Step` enum and `expect` | a `handles` map synced in 3 places (queue.rs:153,218,304); a pointless `name_for_errors` alias (request.rs:244) |
| RO | 3 | 5 | RO/src/service.rs:130 one `resolve_dead_letter`; `for_new_queue` reuses `apply` | RO/src/reply.rs:12 four one-field reply wrappers, each with a single-use `new` | two-pass receive (queue.rs:352-409); `matches_create` clones with `dead_letter` blanked (attributes.rs:227) |
| NA | 3 | 7 | NA/src/queue.rs:243 `send` is one flat function; 5 modules fit the size | NA/src/command.rs:328 a second attribute-name list beside the match at :306 | `swept_at` memo plus `invalidate_sweep` (queue.rs:180,201); dead-letter checks copied (broker.rs:113 vs 145) |
| WI | 2 | 8 | WI/src/broker.rs:65 direct dispatch; smallest crate; split into `broker/` fits | WI/src/broker/message_commands.rs:56 `require` then `.string()?.expect("required")`, copied into every handler | dedup-pruning field (queue.rs:81,255); dead `if let` (message_commands.rs:196) |
| VU | 2 | 9 | VU/src/types.rs:14 one `bounded_int!` serves six range types | VU/src/request.rs:91 a `Command` enum copies the names next to `Request` | `require` plus `required_string` in every arm (request.rs:278); `reconfigure` expires twice (queue.rs:203) |
| BO | 2 | 10 | BO/src/queue.rs:134 `Message::phase` is the only classifier | BO/src/service.rs:134 dead-letter target block copied at :164-176 | `set_attributes` looks up `self.queue(name)` three times (service.rs:197); double map lookup (service.rs:91-103) |
| JU | 2 | 11 | JU/src/request.rs:208 `required<const N>` returns the values | JU/src/queue.rs:249 `matches_request` only forwards | `SequenceNumber::advance` hand-copies the `counter_id!` macro (ids.rs:74); 9 modules for ~1600 lines |
| LE | 2 | 12 | LE/src/attributes.rs:50 one loop with a match parses attributes | LE/src/broker.rs:123 dead-letter block copied at :148-158 | `duplicate` flag splits the id decision between queue.rs:84 and broker.rs:195; `take()` written twice (names.rs:96,134) |
| QE | 2 | 13 | QE/src/queue.rs:289 one `expire` over a `by_sent_at` index; smallest src | QE/src/message_commands.rs:18 `require` then `.expect("required")`, copied about 14 times | a `Dedup` struct with `Rc<str>` only for pruning (queue.rs:211); double `expire` in set (queue_commands.rs:87,113) |
| KE | 2 | 14 | KE/src/service.rs:359 `override_settings` shared by create and set | KE/src/queue.rs:385 two-pass receive with `Step`; in-flight state stored twice (:104-106) | 5 overlapping `Params` helpers (command.rs:221-298); 9 modules |
| FO | 2 | 14 | FO/src/store.rs:264 the new `Config` is built in one expression | FO/src/fifo.rs:46 a dedup eviction index the spec does not need | single-use generic closures (queue.rs:211,315); `blocked` inserted and then removed (queue.rs:338-346) |
| ZU | 2 | 16 | ZU/src/attributes.rs:57 `kind_for_new_queue` holds the Fifo/CBD rule once | ZU/src/queue/mod.rs:108 wake heap, expiry set, ready set and group index; every Tick advances all queues (broker.rs:99) | `require` then `required_*` (request.rs:248,258,305); defensive `if let` (queue/mod.rs:361) |
| ZE | 2 | 17 | ZE/src/queue.rs:462 `enter`/`leave` keep index updates in one place | ZE/src/queue.rs:152 six indexes plus a `Deduplication` queue (:116-149) | `DeduplicationKey` beside `DeduplicationId` (types.rs:153); 30-line json.rs module |
| MO | 2 | 18 | MO/src/attributes.rs:49 `apply` + `Queue::update` is the only update path | MO/src/types.rs:32 `Ranged` trait and macros carry unread `NAME`/`DEFAULT` items (:35,63,85,87) | `receipts` map synced in 4 places and rechecked (queue.rs:207-399); kind matched twice (broker.rs:309-341) |
| VA | 2 | 19 | VA/src/message.rs:120 single `state()`; struct update in `into_dead_letter` (:167) | VA/src/broker.rs:162 `require` then `required_string` with a dead default branch, in every handler | `Command` enum copies names (broker.rs:23-53); a `Counter` with 3 `next_*` (ids.rs:43); ints classified via `to_string` (params.rs:81) |
| MA | 1 | 20 | MA/src/queue.rs:270 one `expire` reused by `push_dead_letter` (:343) | MA/src/handler.rs:105 `require` then `required_string` on the same names, in every handler | defaults applied in 2 places (attributes.rs:82,104); plan/scan receive (queue.rs:348-400); impossible-state branches (broker.rs:314,352); dead-letter lookup copied (broker.rs:120 vs 166) |
| XI | 1 | 21 | XI/src/deliveries.rs:18 a Vec handle registry replaces per-queue maps | XI/src/types.rs:49 identical `Seconds<MIN,MAX>`/`Count<MIN,MAX>` with parallel helpers (request.rs:116-128) | `QueueType` duplicates `Kind` (queue.rs:27,107); attributes pass through 5 structs; limit check in both arms (queue.rs:449,485) |

## What separates the top from the bottom
- **Required parameters.** The top crates have one function that returns the value or `MissingParameter` (SU/src/request.rs:68, JU/src/request.rs:208). The lower ones check presence with `require(&[..])` and then fetch with a second `expect`/`required_*`. That pair is copied into every handler (QE/src/message_commands.rs:18, MA/src/handler.rs:105, VA/src/broker.rs:162).
- **Shared dead-letter validation.** FA, BU, RA, RO and SU validate the dead-letter target in one helper used by both create and set (FA/src/broker.rs:130). BO, LE, MA, MO and NA copy the block (BO/src/service.rs:134 vs :164).
- **Receive shape.** The better crates receive in one direct loop (FA/src/queue.rs:309, NA). Many build a plan of `Step`/`Verdict`/`Action` values and replay it with `expect` (RA/src/queue.rs:276, MA/src/queue.rs:348, MO, QE, RO, KE, LE).
- **Extra indexes.** The bottom adds indexes and memo fields beyond the spec's "seconds, not minutes" requirement, and each must be kept in sync: ZE/src/queue.rs:152 (six indexes), ZU/src/queue/mod.rs:108, `receipts` synced in four places (MO/src/queue.rs:207-378), `swept_at` (NA/src/queue.rs:180, SU/src/queue.rs:140).
- **Duplicated types.** At the bottom, the same concept gets two types: XI/src/types.rs:49 (`Seconds` vs `Count`), XI/src/queue.rs:27 (`QueueType` vs `Kind`), a `Command` enum mirroring `Request` (BU/src/request.rs:21, VU/src/request.rs:91, VA/src/broker.rs:23, LO/src/request.rs:21), and LO's four attribute structs.

## Patterns seen in most crates
- **Good:**
  - No `#[allow]`, `dbg!`, `todo!`, leftover prints or unused functions in any crate.
  - Almost every crate decides message state in one method (`state`/`phase`/`Hold`).
  - Retention runs through one lookup-and-expire helper.
- **Bad:**
  - Nearly all repeat the "ContentBasedDeduplication needs a FIFO queue" check in CreateQueue and in SetQueueAttributes, and many also apply defaults in two places (create vs the idempotence `matches`).
  - Id and handle newtypes repeat their Display/Serialize code by hand, or through small macros and traits that serve only two or three types.
  - Single-field reply wrapper structs, each with a constructor used once, are common (RO, FO, JU).
  - Several crates use 8 or 9 modules for about 1500 lines.
