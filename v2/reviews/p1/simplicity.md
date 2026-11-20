# p1 / simplicity

## Method

There is no `BASE` directory, so I scored every crate as a whole. For each of the 15 crates I read every
non-test line under `src/`. A script printed each `src/**/*.rs` file, stopped at the first top-level
`#[cfg(test)]`, and skipped `src/broker/tests.rs`, blank lines and comment-only lines. Original line
numbers were kept, so the evidence below points at the real files. I did not read `tests/` (not this
dimension). I ran one grep over every `src/` for `#[allow`, `dbg!`, `todo!`, `unimplemented!`,
`eprintln!`, `trait`, `impl<`, `Builder` and `dead_code`. Every crate went through the same pipeline, in
alphabetical order. For each crate I checked:
- request parsing: is "required" decided once or twice?
- CreateQueue vs SetQueueAttributes: are the validations copied?
- the queue's data structures and how many indexes have to be kept in sync
- helper and type count against the number of users
- the receive loop's shape

## Scores

| code | score | rank | best evidence | worst evidence | reason |
|---|---|---|---|---|---|
| BI | 4 | 1 | BI/src/request.rs:149 | BI/src/service.rs:173 | Direct code with few types. `require` returns the value, so nothing is checked twice. Lapses: the CBD and DLQ checks are copied from create (119, 137), and set looks the queue up twice (166, 191) |
| VU | 3 | 2 | VU/src/request.rs:218 | VU/src/broker.rs:26 | Smallest crate, and parsing takes each value by move. `new()` and `Default` disagree on `next_message_id`. `Arc<str>` group ids have no reason to exist (queue.rs:35) |
| WU | 3 | 2 | WU/src/params.rs:26 | WU/src/params.rs:66 | `required([..])` returns the values, so presence is decided once. Integers go through a `to_string().parse()` detour, and `string` matches every `Value` variant (params.rs:47) |
| TO | 3 | 2 | TO/src/broker.rs:127 | TO/src/request.rs:161 | One shared helper checks the DLQ target. Plain `BTreeMap` queue. `require_all` is followed by `required` in 4 handlers but not in the others |
| BO | 2 | 5 | BO/src/params.rs:42 | BO/src/queue_commands.rs:115 | `params.require(..)` is followed by `.expect("required")` in every handler (message_commands.rs:19). SetQueueAttributes expires three times (87, 115, 117). Rc plus VecDeque dedup eviction that one rule does not need |
| CE | 2 | 5 | CE/src/broker.rs:413 | CE/src/broker.rs:158 | Error helpers are shared, but set looks the queue up three times (158, 159, 176). `require` is followed by `expect` (request.rs:112, 142). Each handle is tracked in a map and also on the message |
| FI | 2 | 5 | FI/src/broker.rs:329 | FI/src/queue.rs:115 | `with_delivery` removes a copied block. But the queue keeps 6 indexes in sync (ready, hidden heap, groups, handles, by_sent, dedup) for a scan-sized problem. `UnpairedDeadLetter` has a single use |
| FU | 2 | 5 | FU/src/service.rs:171 | FU/src/command.rs:166 | `require` is followed by `present` for the same field. 11 modules. The `Availability` and `State` enums run in parallel. `QueueKind` forces special patching on Set (service.rs:158) |
| LI | 2 | 5 | LI/src/params.rs:94 | LI/src/command.rs:124 | `require` is followed by `required_string` everywhere. Integers are stringified and re-parsed (params.rs:116). Set expires twice (broker.rs:153, 186) |
| MI | 2 | 5 | MI/src/command.rs:218 | MI/src/queue.rs:72 | The DLQ pair is validated in one place and silently dropped in another. `swept_at` and `invalidate_sweep` are extra state. A closure is defined inside the receive loop (331), next to a three-way `remains` flag |
| PO | 2 | 5 | PO/src/naming.rs:11 | PO/src/broker/queue_commands.rs:47 | `dead_letter()?` is called only to validate, and `apply` derives it again (attributes.rs:84). `require` is followed by `expect` (message_commands.rs:57). Dedup pruning uses a heuristic counter (queue.rs:255) |
| TA | 2 | 5 | TA/src/request.rs:191 | TA/src/queue.rs:135 | Parsing is clean. The queue keeps visible, hidden, by_sent_at, groups and by_handle indexes in sync. `set_config` syncs twice (171). The "unsupported" decision tests fifo twice (queue.rs:262, 268) |
| ZE | 2 | 5 | ZE/src/request.rs:273 | ZE/src/request.rs:77 | Leftover members flag unknown attributes neatly. But every parse arm runs `require` and then `required_string`. `accept_moved` has an odd `else { added += 1 }` (queue.rs:386) |
| HE | 1 | 14 | HE/src/broker.rs:197 | HE/src/request.rs:21 | A `CommandName` enum duplicates the string dispatch it replaces (36, 144). `Requested`, `DeadLetterRequest` and `Update` each serve one site. `take_next` is copied into three types (values.rs:160, 181, 213). About 10 getters with one user each. `Kind` and `QueueType` are parallel enums |
| QO | 1 | 14 | QO/src/broker.rs:191 | QO/src/types.rs:70 | `Count` is a copy of `Seconds` (49). `succ()` is copied into 4 types. `QueueSpec`, `DeadLetterRequest`, `Update` and `Settings` all describe one config. `Outcome` wraps one struct per variant. A `#[cfg(test)]` accessor ships in production code (error.rs:35) |

## What separates the top from the bottom

- **Deciding "required" once.** BI (request.rs:149), VU, WU (params.rs:26), TA, HE, MI, FI and QO return
  the value from the presence check. BO, CE, PO, LI, ZE and FU check presence first and then fetch the
  value again. That fetch ends in `.expect("required")` (BO message_commands.rs:19, PO
  message_commands.rs:57) or in an `ok_or_else(missing)` that can never fire (LI command.rs:124, ZE
  request.rs:77).
- **Queue machinery that fits the size of the problem.** BI, VU, TO and ZE keep one ordered map, plus at
  most a by-age set. FI (queue.rs:115) and TA (queue.rs:135) keep 5 to 6 indexes in sync by hand, with
  heaps, ready sets and group maps. That is far more machinery than "examine in arrival order" calls for.
- **Types without a second user.** HE and QO each add a family of single-use types: a request type, an
  update type, a settings type and a spec type for the same config, and wrapper structs per response
  variant. They also copy boilerplate across newtypes (HE values.rs:160, 181, 213; QO types.rs:49 vs 70).
  BI covers the same ground with one `Config` and one `Attributes`.
- **Repeated state work.** BO expires three times in one handler (queue_commands.rs:87, 115, 117), TA
  syncs twice (queue.rs:171-174), and CE looks up the same queue three times (broker.rs:158-176).
  BI and VU do each step once, apart from a second lookup.
- **Validate here, ignore there.** MI checks the dead-letter pair (broker.rs:347), and then
  `dead_letter_of` maps a half pair to `None` (queue.rs:72). PO calls `update.dead_letter()?` only for its
  error (queue_commands.rs:47). In both cases one decision is spread over two places that can drift apart.

## Patterns seen in most crates

- Nearly every crate copies the ContentBasedDeduplication check and the dead-letter-target block between
  CreateQueue and SetQueueAttributes. Only TO, FU, HE and QO pull the target check into a shared helper.
- No crate uses `#[allow]`, `dbg!` or `todo!`, or keeps dead code. The only `eprintln!` calls are
  legitimate I/O-failure reports in `main`.
- Most crates hand-write the same `ErrorCode::as_str` match that a `Serialize` or `Debug` derive gives
  for free (BI, FU, HE, QO, TO and VU use the derive).
- Many crates split receive into a plan pass and an apply pass, with a one-use `Step` or `Action` enum.
  The split is defensible because of the borrow checker, but where the crate also adds a separate
  `Received` or `Delivery` type it adds a layer.
