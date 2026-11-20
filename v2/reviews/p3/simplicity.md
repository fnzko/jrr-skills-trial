# p3 / simplicity

## Method

I read all of BASE/src (1,300 lines) first and listed its simplicity lapses:

- L1: the `ContentBasedDeduplication requires FIFO` block is copied into both `create_queue` and `set_queue_attributes` (BASE/src/broker/queue_commands.rs:42, :95)
- L2: the dead-letter pairing is decided twice. `update.dead_letter()?` is called only to validate and its result is thrown away (queue_commands.rs:47), then `Attributes::apply` pairs the two fields again (attributes.rs:84)
- L3: `delete_message` and `change_message_visibility` look the queue up in `in_flight_message` and then again with `queue_mut`, followed by an `if let Some` on a lookup that cannot fail (message_commands.rs:181, :194-196)
- L4: a `missing` helper duplicates the `MissingParameter` text that `require` builds (message_commands.rs:25 vs request.rs:69)
- L5: same-named `invalid_value` helpers in two modules, plus a local `unsupported` (request.rs:48, queue_commands.rs:14, message_commands.rs:21)

For each of the 18 attempts I then read the complete `diff -ru BASE/src X/src` and `diff -ru BASE/tests X/tests`, plus every new file in full (ids.rs, time.rs). I asked the same questions of every crate: which of L1-L5 did the change remove, and what new abstraction, helper, type or copied block did it add, and how many users does each new item have. I ran one grep over every crate for `#[allow]`, `dbg!`, `println!`, `todo!` and `dead_code`. It found nothing in any of them. A second grep, also over every crate, collected the line numbers for the evidence below. No crate got a deeper read than its diff.

No crate reaches 4: every attempt keeps at least two of the BASE lapses. No crate gets 0 either: even the heaviest rewrites are readable and contain no dead code.

## Scores

| code | score | rank | best evidence | worst evidence | reason |
|---|---|---|---|---|---|
| ZA | 3 | 1 | ZA/src/broker/queue_commands.rs:43 (dead letter resolved once, passed to `apply` at :46) | ZA/src/broker/message_commands.rs:179 (now/issued/lookup preamble repeated at :193) | Removes L1, L2, L3 and L4 with plain code; only the handler preamble and L5 remain |
| FE | 3 | 2 | FE/src/broker/message_commands.rs:197 (`in_flight_message` returns the queue, so there is no second lookup) | FE/src/broker/queue_commands.rs:44 (CBD block still copied at :97) | Fixes L2, L3 and L4 and drops the unused `Params::has`; L1 remains |
| PI | 3 | 2 | PI/src/broker/queue_commands.rs:55 (dead letter once, into `apply` at :58) | PI/src/broker/message_commands.rs:190 (double lookup plus `if let` kept) | Fixes L1, L2 and L4; L3 and L5 remain |
| HO | 2 | 4 | HO/src/attributes.rs:135 (`check_deduplication_fits`, two callers) | HO/src/broker/message_commands.rs:193 (double lookup kept) | Fixes L1 and L4 only; L2, L3 and L5 untouched |
| JI | 2 | 4 | JI/src/attributes.rs:129 (one CBD check, two callers) | JI/src/broker/message_commands.rs:193 (double lookup kept) | Same scope as HO |
| ME | 2 | 4 | ME/src/broker/queue_commands.rs:18 (CBD check deduplicated) | ME/src/broker/message_commands.rs:191 (double lookup kept) | L1 and L4 fixed; `required_*` still re-checks what `require` checked |
| TE | 2 | 4 | TE/src/broker/queue_commands.rs:19 (CBD check deduplicated) | TE/src/broker/message_commands.rs:190 (double lookup kept) | Same as ME |
| VA | 2 | 8 | VA/src/error.rs:54 (one `missing_parameter`) | VA/src/broker/queue_commands.rs:18 (an `impl AttributeUpdate` placed in the handler module, plus a second `impl Error` block) | L1 and L4 fixed, but the shared check lives away from its type |
| FO | 2 | 9 | FO/src/broker/queue_commands.rs:18 (CBD check deduplicated) | FO/src/broker/message_commands.rs:26 (`check_body_len` has one caller) | L1 and L4 fixed; adds a single-use helper |
| WO | 2 | 10 | WO/src/request.rs:49 (`missing_parameter` shared with `require`) | WO/src/broker/queue_commands.rs:44 (CBD block still copied at :97) | Smallest change: fixes L4 and adds constants; everything else is as in BASE |
| NU | 2 | 11 | NU/src/error.rs:52 (one set of error constructors; three local helpers gone) | NU/src/broker/message_commands.rs:40 (`fifo_ids`, five parameters, plus a `FifoIds` struct, for one caller) | Fixes L4 and L5; adds single-use machinery; L1 kept |
| BASE | 2 | 12 | BASE/src/queue.rs:194 (`plan_receive` is pure and direct) | BASE/src/broker/queue_commands.rs:42 (copied CBD block, again at :95) | L1-L5 all present, but no needless abstraction |
| PO | 2 | 13 | PO/src/error.rs:55 (central `invalid_value`, `missing` and `unsupported`) | PO/src/broker/message_commands.rs:48 (`FifoSend` enum plus a six-argument `resolve_fifo_ids` for one caller) | Fixes L4 and L5; adds a one-caller enum and helper; L1 kept |
| FI | 2 | 13 | FI/src/error.rs:58 (`Error::invalid_value` replaces two helpers) | FI/src/broker/message_commands.rs:62 (six-argument `check_fifo` plus `FifoSend` at :53, one caller) | Same trade as PO |
| SU | 2 | 15 | SU/src/attributes.rs:136 (one CBD check) | SU/src/broker/message_commands.rs:53 (`FifoIds`, `FifoSend` at :58 and `fifo_send_ids` at :67: three items for one call site) | Fixes L1 and L4 but adds the most single-use machinery among the light edits |
| VO | 2 | 16 | VO/src/error.rs:55 (named constructors including `queue_does_not_exist`) | VO/src/broker/message_commands.rs:48 (`Send` param struct, `FifoIds` and `Admission` at :62 for one `admit` call) | Fixes L4 and L5; three new types around one call |
| QO | 1 | 17 | QO/src/broker/message_commands.rs:198 (`queue_and_handle` plus `remove_in_flight` remove L3) | QO/src/ids.rs:14 (`take_next` copied at :36); QO/src/request.rs:32 (`Command::from_name` repeats the dispatch list) | Type-heavy rewrite that keeps L1 and L2; `move_to_dead_letter_queue` (message_commands.rs:153) looks up the source queue again |
| SI | 1 | 18 | SI/src/request.rs:168 (`require` returns the values, so presence is decided once) | SI/src/queue.rs:24 (`Delivery` enum needs three accessor matches at :65-81); request.rs:169 (static `NULL` placeholder) | Many new types (Delivery, NewMessage, Param, Command, Arc<str> names); L1 and L3 kept |
| CU | 1 | 18 | CU/src/queue.rs:140 (`expire` becomes one direct `retain`) | CU/src/attributes.rs:38 (`QueueKind`, which makes `apply` branch at :111); request.rs:94 (static `ABSENT` placeholder) | Largest diff (+840 lines): time.rs, ids.rs (three copied `next` impls), QueueName and Required; L1 and L2 kept |

## What separates the top from the bottom

- **Removing a decision made twice instead of moving it.** Only ZA, FE and PI stop calling `update.dead_letter()` just to discard the result. They pass the resolved value into `apply` instead (ZA/src/broker/queue_commands.rs:43-46, PI/...:55-58). Every other crate still pairs the dead-letter fields in two places (e.g. HO/src/broker/queue_commands.rs:43).
- **Removing the double queue lookup.** ZA, FE, QO and CU return or pass the queue, so delete and visibility changes no longer do two lookups with an impossible `if let` (FE/src/broker/message_commands.rs:197). The others keep BASE's pattern (e.g. PO/src/broker/message_commands.rs:216).
- **Adding one-caller abstractions around `send_message`.** NU, PO, FI, SU, VO and SI each pull the FIFO checks into a helper with five or six parameters. Most also wrap the result in a new enum or struct (VO/src/broker/message_commands.rs:48-71; SU/...:53-67). The handler gets no shorter and gains a type to learn.
- **Type rewrites add code without removing duplication.** QO, SI and CU introduce ids/time modules and a `Command` enum whose `from_name` repeats the dispatch match (CU/src/request.rs:32, QO/src/request.rs:32). They also copy counter methods (QO/src/ids.rs:14/:36; CU ids.rs:13/:31/:58). Yet they leave the copied CBD block in place (CU/src/broker/queue_commands.rs:44/:97).
- **Placeholder tricks.** SI and CU make `require` return typed values, which is a real simplification. But both need a static `Null` sentinel to fill the array before every slot is checked (SI/src/request.rs:169, CU/src/request.rs:94).

## Patterns seen in most crates

- 17 of 18 attempts route `MissingParameter` through one constructor (L4); only CU keeps a local `missing` beside its new `require`. It was the easiest duplicate to spot.
- Only 9 of 18 deduplicate the CBD check (L1): HO, JI, ME, TE, VA, FO, SU, PI and ZA. In CU, SI and QO the copied block survives a large rewrite.
- About half (FE, PI, ME, TE, ZA, QO, SI, CU) replace `string(..)?.expect("required")` with a `required_*` helper or a typed `require`. Only SI and CU's typed `require` removes the second presence check; `required_*` after `require` still checks twice.
- Most attempts name the shared ranges as constants (`MAX_VISIBILITY_TIMEOUT`, `DELAY_RANGE`), a small, consistent gain.
- The duplicated local `invalid_value` (L5) survives in every crate except NU, PO, FI and VO.
- None adds `#[allow]`, debug output or dead code. Module layout stays at BASE's eight files except QO (+ids.rs), SI (+ids.rs) and CU (+ids.rs, +time.rs).
