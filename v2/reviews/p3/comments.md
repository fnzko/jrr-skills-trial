# p3 / comments
## Method
- Same steps for every crate, BASE included. First, `grep` for every line that starts with `//`, `///`, `//!` or `/*` in `src/` and `tests/`, and count them against total lines.
- Second, `diff -r BASE <CODE>`, filtered to comment lines, plus the new files (CU `ids.rs`/`time.rs`, QO `ids.rs`, SI `ids.rs`), read in full for comments. Each added or changed comment was read with the item it documents.
- Third, the same searches in every crate for trailing comments, banners, step numbers, TODO, and history or plan wording ("previously", "no longer", "refactor").
- Fourth, a stale-doc check on the two items most attempts changed: `AttributeSet::apply` and `Params::require`. The question was whether the doc still matches the new signature.
- Fifth, a count of undocumented `pub` items, as a check on how much is documented.
- Code with no comments was not read.

BASE is already good: module docs give reasons, field docs give invariants, and the test comments explain why. Its few lapses are restatements such as `broker.rs:117`. Every attempt keeps those, so attempts are told apart by the comments they added or changed.

## Scores
| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| BASE | 3 | 5 | BASE/src/queue.rs:197 | BASE/src/broker.rs:117 | Reason and invariant comments throughout; a few restate the code |
| CU | 3 | 5 | CU/src/ids.rs:1 | CU/src/request.rs:160 | Good rationale on new types, but the `require` doc was left stale after it started returning values, and `request.rs:93` reads oddly |
| FE | 4 | 1 | FE/src/queue.rs:189 | FE/src/request.rs:77 | Updated the `apply` contract for the new parameter and added a real precondition on `key` |
| FI | 2 | 19 | FI/src/queue.rs:14 | FI/src/queue.rs:83 | Docs on nearly every item; several restate the name (`queue.rs:83`, `:112`, `:137`) and add volume, not information |
| FO | 3 | 5 | FO/src/queue.rs:107 | FO/src/broker.rs:117 | Few additions, each one a reason; inherits BASE's lapses |
| HO | 3 | 5 | HO/src/attributes.rs:133 | HO/src/error.rs:54 | Few, useful contract docs; the error constructor doc mostly restates |
| JI | 3 | 5 | JI/src/attributes.rs:18 | JI/src/error.rs:52 | Small additions; the constructor doc restates `missing` |
| ME | 3 | 5 | ME/src/request.rs:81 | ME/src/broker.rs:117 | Call-order contract on the new getters; otherwise BASE |
| NU | 3 | 5 | NU/src/broker/message_commands.rs:38 | NU/src/broker.rs:117 | Return contract on the new FIFO check; otherwise BASE |
| PI | 4 | 1 | PI/src/attributes.rs:68 | PI/src/broker.rs:117 | Brought the changed `apply` doc in line and says why it takes `dead_letter`; the getters say what they assume about `require` |
| PO | 3 | 5 | PO/src/broker/message_commands.rs:56 | PO/src/error.rs:53 | New enum and function documented by contract; the section doc is filler |
| QO | 3 | 5 | QO/src/broker/message_commands.rs:155 | QO/src/broker/message_commands.rs:217 | Explains why a clone is needed and the id ordering invariant; the bare "SPEC section 6, rule 2." doc says nothing |
| SI | 4 | 1 | SI/src/attributes.rs:93 | SI/src/ids.rs:30 | Explains the `Partial` state and the `Rc` sharing; the `require` doc was updated to its new return value |
| SU | 3 | 5 | SU/src/broker/message_commands.rs:65 | SU/src/request.rs:55 | Good default-id contract; the constructor doc restates |
| TE | 3 | 5 | TE/src/request.rs:87 | TE/src/broker.rs:117 | One useful contract (absent is still reported); otherwise BASE |
| VA | 3 | 5 | VA/src/broker/queue_commands.rs:19 | VA/src/broker.rs:117 | Minimal, accurate additions |
| VO | 3 | 5 | VO/src/broker/message_commands.rs:69 | VO/src/error.rs:53 | New types documented by contract; the section doc is filler |
| WO | 3 | 5 | WO/src/request.rs:48 | WO/src/broker.rs:117 | Almost no change from BASE |
| ZA | 4 | 1 | ZA/src/broker/message_commands.rs:206 | ZA/src/broker.rs:117 | Added the `issued` invariant and updated the `apply` contract to its new parameter |

## What separates the top from the bottom
- The top four kept docs true when they changed a signature: FE/src/attributes.rs:66, PI/src/attributes.rs:66, ZA/src/attributes.rs:71 and SI/src/request.rs:166. CU changed `require` to return values and left "Fails with ... first absent name" as its only doc (CU/src/request.rs:160).
- The best additions state preconditions or invariants the type cannot express:
  - FE/src/queue.rs:189: `key` is only valid if nothing changed in between.
  - ZA/src/broker/message_commands.rs:206: every smaller handle number has been issued.
  - QO/src/ids.rs:27: handle order equals issue order.
- Rationale for a non-obvious line or structure is rare and valuable: QO/src/broker/message_commands.rs:155 (why the name is copied) and SI/src/attributes.rs:23 (why the name is shared).
- FI is lowest because it documents everything, including the obvious. "The messages and settings of one queue" (FI/src/queue.rs:83), "The number of messages held" (`:112`) and "Counts the messages by state" (`:137`) restate the names. It has the most comment lines among the minimal-diff crates.
- Error-constructor docs are the most common filler: HO/src/error.rs:54, JI/src/error.rs:52, SU/src/request.rs:55, WO/src/request.rs:48, PO/src/error.rs:53, VO/src/error.rs:53.

## Patterns seen in most crates
- No banners, step numbers, TODOs, history or plan talk anywhere. The only trailing comments are BASE's `// r-1` markers in a test, kept everywhere.
- Every crate keeps BASE's few restating docs (e.g. `broker.rs` "Builds a result member map"). None removed a weak comment.
- Most attempts added docs only on the constants they extracted (`MAX_DELAY_SECONDS`, `DELAY_RANGE`). Those docs usefully say the bound is shared by the queue attribute and the per-request value.
- Comment volume stays proportionate (about 105 to 139 comment lines in 2300 to 2700 total). The spread between attempts is narrow, since most changed only a handful of comments.
