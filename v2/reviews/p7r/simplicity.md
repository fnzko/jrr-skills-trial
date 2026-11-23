# p7r / simplicity

## Method

I read all of `BASE/src`. For each of the 21 attempts I read the full `diff -ruN BASE/src <CODE>/src`. No attempt changed `tests/`. I ran the same greps on every crate, BASE included:
- the duplicated `ContentBasedDeduplication requires a FIFO queue` check in create/set;
- the double queue lookup in `delete_message` / `change_message_visibility` (`in_flight_message` followed by `queue_mut`);
- `expect("required")` after `require`;
- duplicated error helpers (`fn missing`, `fn invalid_value`);
- `#[allow]`, `dbg!`, `println!`.

None of the crates has an `#[allow]` or leftover debugging. I scored each attempt mainly on what its diff adds or removes relative to BASE. The duplicated dead-letter target validation in create/set is left in every crate, so it does not separate them.

## Scores

| code | score | rank | best evidence | worst evidence | reason |
|---|---|---|---|---|---|
| RO | 3 | 1 | RO/src/attributes.rs:134 | RO/src/broker/message_commands.rs:206 | Moves the copied CBD check into one helper and extracts the FIFO rules; long `expect` messages are repeated at each call site and the double lookup stays |
| WE | 3 | 1 | WE/src/attributes.rs:75 | WE/src/broker/message_commands.rs:203 | Removes the dead-letter pairing that was decided twice, the double lookups and the double lookup in `delete_queue`; a 4-line lookup/parse block is now copied into two handlers, plus a single-use `Message::new` |
| WO | 3 | 1 | WO/src/broker/message_commands.rs:210 | WO/src/broker/queue_commands.rs:101 | `queue_and_handle` ends the double lookup, `delete_queue` removes the queue directly, the local handle counter is gone; the CBD check is still copied and the `expect` messages are verbose |
| XU | 3 | 1 | XU/src/attributes.rs:126 | XU/src/broker/queue_commands.rs:21 | A small diff that only removes things (CBD helper, shared `missing_parameter`, `Command` enum); BASE's `require` plus `expect("required")` stays |
| BASE | 2 | 5 | BASE/src/queue.rs:194 | BASE/src/broker/message_commands.rs:182 | Clean receive planning, but the CBD check is copied, the queue is looked up twice per handle command, and presence is checked twice (`require` then `expect`) |
| CA | 2 | 5 | CA/src/request.rs:125 | CA/src/broker/message_commands.rs:35 | `required_*` removes the expects; adds a single-use `check_body_len`, and the CBD copy and double lookup stay |
| CE | 2 | 5 | CE/src/error.rs:55 | CE/src/broker/message_commands.rs:177 | One `Error::missing`, FIFO rules in one function; the double lookup and the CBD copy stay |
| DA | 2 | 5 | DA/src/broker/message_commands.rs:60 | DA/src/broker/queue_commands.rs:97 | `required_*` and `fifo_ids` help; adds a `FifoIds` struct and range constants; the CBD copy stays |
| GU | 2 | 5 | GU/src/broker/message_commands.rs:198 | GU/src/broker/message_commands.rs:153 | Ends the double lookup, but the new dead-letter helper looks the source queue up again; `take_next` is copied into two id types (GU/src/ids.rs:14, :36) |
| HE | 2 | 5 | HE/src/error.rs:53 | HE/src/broker/message_commands.rs:53 | Shared error constructors; adds a single-use `FifoSend` enum and a 6-argument `check_fifo`; the CBD copy stays |
| JO | 2 | 5 | JO/src/queue.rs:191 | JO/src/request.rs:73 | An impossible `if let` branch becomes an expect, but the command name is kept as a `String` and parsed again at dispatch; double lookup |
| NE | 2 | 5 | NE/src/error.rs:55 | NE/src/broker/message_commands.rs:57 | Shared error constructors; a single-use enum plus a 6-argument resolver; the CBD copy stays |
| RU | 2 | 5 | RU/src/request.rs:122 | RU/src/broker/queue_commands.rs:97 | Small and clean (`required_*`, `Command`, `fifo_ids`), but it leaves BASE's duplications as they were |
| SA | 2 | 5 | SA/src/attributes.rs:136 | SA/src/broker/message_commands.rs:58 | CBD helper; adds a `FifoIds` struct plus a `FifoSend` enum for one caller; still `require` plus `expect` |
| VA | 2 | 5 | VA/src/ids.rs:33 | VA/src/ids.rs:42 | Handle parsing moves to its type; adds a one-line single-use `is_issued_before` and trivial `new` constructors; double lookup |
| VI | 2 | 5 | VI/src/broker/message_commands.rs:188 | VI/src/broker/message_commands.rs:32 | Silent impossible branches become expects and error constructors are shared; a `SendRequest` struct exists for one handler only; the CBD copy stays |
| VO | 2 | 5 | VO/src/broker/queue_commands.rs:19 | VO/src/queue.rs:157 | CBD check written once; `push` is split into `push`, `push_fifo` and `insert` with a `debug_assert` (VO/src/queue.rs:151), plus a separate `limits` module |
| ZE | 2 | 5 | ZE/src/broker/message_commands.rs:170 | ZE/src/ids.rs:46 | Issuing a handle no longer needs a local counter, but `MessageIds` and `ReceiptHandles` are copied blocks (ids.rs:19, :46); a single-use `SendRules` enum |
| ZO | 2 | 5 | ZO/src/request.rs:117 | ZO/src/broker/queue_commands.rs:19 | `required_*` and `Command`; `queue_name` checks absence twice (`require`, then `required_string`); the CBD copy stays |
| HO | 1 | 20 | HO/src/request.rs:168 | HO/src/attributes.rs:24 | `require` returning the values is a real single source, but it brings an `Arc<str>` to avoid one clone, a counter macro (HO/src/ids.rs:10), a static NULL placeholder (request.rs:169), and a `Delivery` enum with three accessors (queue.rs:65) |
| JA | 1 | 20 | JA/src/broker/message_commands.rs:193 | JA/src/request.rs:94 | Returning the queue ends the double lookup; adds a time module, a `QueueKind` with accessors and a rebuilt `if` in `apply` (attributes.rs:111), `QueueName`, `DeadLetterUpdate`, and a `Required` type with an ABSENT placeholder; far more machinery than this problem needs |
| LI | 1 | 20 | LI/src/request.rs:122 | LI/src/queue.rs:219 | The shared `find_in_flight` is replaced by two hand-written copies (`delete_in_flight` at queue.rs:206, `change_visibility`); `fn missing` is defined twice (request.rs:95, message_commands.rs:18); still a double lookup through `issued_handle` |

## What separates the top from the bottom

- **Removing a decision made twice.** The top crates each remove one:
  - WE validates the dead-letter pair once and passes it into `apply` (WE/src/attributes.rs:75).
  - RO and XU move the copied CBD check into one helper (RO/src/attributes.rs:134, XU/src/attributes.rs:126).
  - WO and GU end the two queue lookups (WO/src/broker/message_commands.rs:210).

  Most middle crates left all of these in place.
- **Not adding new duplication.** At the bottom, LI turns BASE's one in-flight lookup into two (LI/src/queue.rs:206, :219) and defines `missing` twice. ZE copies a counter struct (ZE/src/ids.rs:19, :46). WE's one regression is the copied lookup/parse block in two handlers (WE/src/broker/message_commands.rs:187, :203).
- **Abstractions sized to the problem.** HO and JA add Arc, a macro, static placeholder values, time newtypes and enum-plus-accessor layers to a 2.4k-line crate (HO/src/attributes.rs:24, JA/src/request.rs:94). The 3-scored crates add almost no new types.
- **Single-use helpers that take the whole handler's state.** A `FifoSend`/`SendRules` enum with a 5-6 argument resolver used by exactly one caller is in NE, HE, SA and ZE (NE/src/broker/message_commands.rs:57). This moves code around rather than simplifying it.
- **Impossible branches.** VI, JO and WO replace BASE's silent `if let Some(message) = ...message_mut(key)` with an expect that states the invariant (VI/src/broker/message_commands.rs:188). This matches the "no branch for a state that cannot occur" criterion.

## Patterns seen in most crates

- Nearly all of them (20 of 21) turn the stringly dispatch into a `Command` enum. Nearly all replace `expect("required")` with `required_string`/`required_integer`, but keep the separate `require` call. Presence is therefore still decided in two places, except in HO and JA.
- 17 of 21 leave the CBD check copied between `create_queue` and `set_queue_attributes`. All 21 leave the duplicated dead-letter target validation.
- 16 of 21 keep the double queue lookup in `delete_message` and `change_message_visibility`.
- Most add an `ids.rs` with `MessageId`/`ReceiptHandle`. Where both types get the same `next`/`take_next`/`FIRST` code, it is copied rather than shared.
- No crate adds `#[allow]`, debugging output or dead code. The module layout stays close to BASE, apart from VO's `limits.rs` and JA's `time.rs`.
