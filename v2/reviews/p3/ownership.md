# p3 / ownership

## Method

I read every file in BASE/src in full and listed the ownership issues it has. For each of the 18
attempts I ran `diff -ru BASE/src X/src` and read every changed hunk that touches allocation,
cloning, borrowing, lookups or scans. I used the same grep on every crate's src/ to keep the method
even. The grep counted `clone()`, `to_owned`, `to_string()`, `collect`, `queue_mut(` and `lookup(`, and
found the line of each known BASE issue: the dead-letter name clone in `receive_message`, the
`Command` string clone in `Request::parse`, the clone in `AttributeUpdate::dead_letter`, the second
`queue_mut` after `in_flight_message`, and the borrowed `Params::string` getter. I read the full
function only where a diff restructured it (CU, QO, SI receive/create; FE, ZA, QO, CU
delete/change). I read no tests for this dimension.

BASE ownership baseline: the parameter layer is good. `Params<'a>` is `Copy`, getters return
`&'a str`, and values are only owned where they are stored (body, group id, queue name on insert).
`expire` skips the scan unless the oldest message has expired. The lapses are:
(a) the dead-letter queue name is cloned on every receive to release the queue borrow
(BASE/src/broker/message_commands.rs:149);
(b) the command name is cloned out of the parsed map only to be matched as a string
(BASE/src/request.rs:31);
(c) `dead_letter()` clones the name even where the result is only validated and dropped
(BASE/src/attributes.rs:129, called at queue_commands.rs:47);
(d) delete and change-visibility look the queue up twice, running `expire` twice
(BASE/src/broker/message_commands.rs:182,196);
(e) `expire` collects keys into a Vec and then removes them one by one.
None is severe, so BASE gets 2 (mixed).

## Scores

| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| CU | 3 | 1 | CU/src/queue.rs:140 | CU/src/broker/message_commands.rs:137 | Fixes (b)(c)(d)(e): single lookup returns `&mut Queue`, `dead_letter()` borrows, `expire` uses one `retain`; keeps the per-receive name clone and allocates `QueueName` before the exists check (queue_commands.rs:27) |
| QO | 3 | 1 | QO/src/broker/message_commands.rs:202 | QO/src/attributes.rs:131 | Fixes (b)(d), and the DLQ name clone now happens only when messages are dead-lettered (:153); keeps (c) and adds a second source lookup in that path |
| SI | 3 | 3 | SI/src/attributes.rs:144 | SI/src/broker/message_commands.rs:183 | Fixes (b)(c), moves dead-letter fields instead of copying them, drops `Clone` on `Message`; per-receive clone becomes `Arc::clone` (:153); double lookup (d) stays |
| FE | 3 | 4 | FE/src/broker/message_commands.rs:197 | FE/src/broker/message_commands.rs:145 | Fixes (d): `in_flight_message` hands back the queue it found; (c) clone is now kept instead of dropped; (a)(b) unchanged |
| ZA | 3 | 4 | ZA/src/broker/message_commands.rs:180 | ZA/src/broker/message_commands.rs:146 | Fixes (d) with one `lookup` per handler and a free `in_flight_message(&Queue, ..)`; (a)(b)(c) unchanged |
| BASE | 2 | 6 | BASE/src/request.rs:79 | BASE/src/broker/message_commands.rs:149 | Baseline: borrowed params, but lapses (a) to (e) |
| FI | 2 | 6 | FI/src/request.rs:73 | FI/src/broker/message_commands.rs:181 | FIFO helper borrows `&Queue` and `&'a str`; no ownership change from BASE |
| FO | 2 | 6 | FO/src/request.rs:76 | FO/src/broker/message_commands.rs:184 | Constants and helpers only; BASE lapses unchanged |
| HO | 2 | 6 | HO/src/request.rs:76 | HO/src/broker/message_commands.rs:146 | Constants and helpers only; BASE lapses unchanged |
| JI | 2 | 6 | JI/src/request.rs:76 | JI/src/broker/message_commands.rs:179 | Constants and helpers only; BASE lapses unchanged |
| ME | 2 | 6 | ME/src/request.rs:92 | ME/src/broker/message_commands.rs:179 | Adds `required_*` getters, still borrowed; BASE lapses unchanged |
| NU | 2 | 6 | NU/src/request.rs:73 | NU/src/broker/message_commands.rs:167 | `FifoIds<'a>` is `Copy` over borrowed strs; BASE lapses unchanged |
| PI | 2 | 6 | PI/src/request.rs:90 | PI/src/attributes.rs:128 | Passes the `dead_letter()` result into `apply`, so the clone is kept and the owned original dropped (no net gain); rest unchanged |
| PO | 2 | 6 | PO/src/request.rs:73 | PO/src/broker/message_commands.rs:202 | FIFO helper borrows; BASE lapses unchanged |
| SU | 2 | 6 | SU/src/request.rs:81 | SU/src/broker/message_commands.rs:212 | Borrowed `FifoIds<'a>`; BASE lapses unchanged |
| TE | 2 | 6 | TE/src/request.rs:80 | TE/src/broker/message_commands.rs:178 | `required_*` getters, borrowed; BASE lapses unchanged |
| VA | 2 | 6 | VA/src/request.rs:76 | VA/src/broker/message_commands.rs:179 | Constants and helpers only; BASE lapses unchanged |
| VO | 2 | 6 | VO/src/request.rs:76 | VO/src/broker/message_commands.rs:223 | `admit` takes `&Queue` and a borrowed `Send<'a>`; BASE lapses unchanged |
| WO | 2 | 6 | WO/src/request.rs:81 | WO/src/broker/message_commands.rs:146 | Constants only; BASE lapses unchanged |

## What separates the top from the bottom

- Removing the double queue lookup in delete/change-visibility. FE (:197), ZA (:180), QO (:202) and
  CU return or reuse the `&mut Queue` found while checking the receipt handle. The other 14 crates
  still call `in_flight_message` and then `self.queue_mut(name)` again, e.g. VO/src/broker/message_commands.rs:223,237.
- Parsing `Command` into an enum at the boundary. CU, QO and SI no longer clone the command `String`
  out of the JSON map to match on it. Every other crate keeps `name.clone()` at request.rs:31.
- `dead_letter()` returning a borrow. SI (attributes.rs:144) and CU (attributes.rs:175) return
  `Option<&DeadLetter>`, so validating no longer allocates. FE, PI and ZA moved the clone instead
  of removing it.
- The dead-letter name clone during receive is still there in every crate. QO limits it to receives
  that actually dead-letter something and says why in a comment. SI makes it cheap with `Arc<str>`,
  which is a workaround rather than a restructure. No crate splits the borrow so that the clone is
  unnecessary.
- CU alone replaces the collect-then-remove loop in `expire` with one `retain` pass that also
  clears handles (CU/src/queue.rs:140). It also adds a small new allocation: `QueueName::parse`
  owns the name before the "already exists" check (CU/src/broker/queue_commands.rs:27).

## Patterns seen in most crates

- Nearly every attempt left ownership alone. 13 of 18 diffs only extract constants, add error
  constructors or move the FIFO checks into helpers. Those helpers consistently borrow
  (`&Queue`, `Option<&'a str>`, `Copy` structs of `&'a str`), so no attempt made ownership worse.
- The borrowed parameter layer from BASE (`Params<'a>: Copy`, `&'a str` getters) survives
  everywhere. Owned `String`s are created only for stored data (body, group id, dedup key, queue name).
- No attempt adds clones to satisfy the borrow checker, sorts or parses again per call, or scans
  a collection inside a loop over itself. The remaining issues are the BASE lapses that the
  attempts did not touch.
