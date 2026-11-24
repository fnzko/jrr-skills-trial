# p14f / tests
## Method
For BASE I read every test file in full (tests/common, protocol, binary, queues, messages) and listed the unit tests in src. For each attempt I ran `diff -r BASE <CODE>`, then read in full every test file it added or changed (batches.rs, tags.rs, max_message_bytes.rs, or the block appended to messages.rs in VO and VU) plus any new `#[cfg(test)]` module in src (KI, PA, RE tags.rs). All twelve attempts changed only one line of queues.rs (MaxMessageBytes in the defaults), and that line counts for none of them. Against SPEC-feature.md I checked: batch rules 1 to 4 and their order, check order before the numbered rules, entries seeing earlier entries (ids, sequence, dedup, used handles), per-entry failures, the all-failed success, the message limit inside a batch, tag key/value/byte rules, the 50-tag limit with atomic failure, tags vs attributes/purge/delete, MaxMessageBytes range, its place before rule 1, UTF-8 bytes, CreateQueue rule 4, existing messages, and dead-letter moves. For assertion quality I looked for status-only checks (`["ok"] == false` with no code), assertions inside loops over results, vague names, misnamed helpers, and comments that do not match the data.

## Scores
| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| BASE | 3 | 5 | BASE/tests/protocol.rs:18 | BASE/tests/binary.rs:29 | exact codes everywhere; binary tests fall back to substring `contains` |
| BA | 3 | 5 | BA/tests/batches.rs:151 | BA/tests/max_message_bytes.rs:20 | precise check-order cases; wrong byte-count comment; no dead-letter test |
| BO | 3 | 5 | BO/tests/batches.rs:62 | BO/tests/max_message_bytes.rs:41 | full (Id, code) lists, dead-letter covered; several ok-false checks with no code |
| KI | 4 | 1 | KI/tests/batches.rs:175 | KI/tests/max_message_bytes.rs:89 | covers rule interplay (too many beats bad id), atomic tag failure, per-queue tags |
| LI | 2 | 12 | LI/tests/batches.rs:118 | LI/tests/batches.rs:126 | test named for the message limit only checks id sharing; vague names (`validation`, `delete_batch`); `Value2` alias |
| PA | 4 | 1 | PA/tests/batches.rs:153 | PA/tests/max_message_bytes.rs:15 | table runs every batch rule on both commands; helper checks messages; no dead-letter test |
| PI | 4 | 1 | PI/tests/batches.rs:271 | PI/tests/max_message_bytes.rs:34 | deep delete-batch coverage (foreign handle); one wrong byte comment |
| QA | 3 | 5 | QA/tests/batches.rs:139 | QA/tests/max_message_bytes.rs:51 | rules checked on both commands; no batch message-limit test; misleading test name |
| QE | 2 | 12 | QE/tests/batches.rs:154 | QE/tests/batches.rs:56 | assertion sits in a loop that may not run; helpers `tag`/`send` assert failure under neutral names; vague names |
| RE | 4 | 1 | RE/tests/batches.rs:158 | RE/tests/batches.rs:330 | exhaustive check-order table with no-op proof, unit test on the tag limit, exact shapes |
| VO | 3 | 5 | VO/tests/batches.rs:109 | VO/tests/batches.rs:239 | broad coverage incl. dead-letter; convoluted, partly redundant delete-batch assertion |
| VU | 3 | 5 | VU/tests/batches.rs:186 | VU/tests/batches.rs:105 | rules on both commands; failed FIFO entries asserted without their Id; no dead-letter test |
| ZA | 3 | 5 | ZA/tests/batches.rs:21 | ZA/tests/batches.rs:107 | helper asserts error member count and message; fill loop unchecked; vague names `delete_batch`, `check_order` |

## What separates the top from the bottom
- Top crates check the batch rules' relative order, not just each rule (KI/tests/batches.rs:175, RE/tests/batches.rs:244, PA/tests/batches.rs:148), and prove a rejected batch did nothing (RE/tests/batches.rs:237).
- Top crates assert the full ordered (Id, code) list of `Failed` (PI/tests/batches.rs:271); weaker ones index `Failed[0]` and `[1]` by position without Ids (VU/tests/batches.rs:105) or loop over it (QE/tests/batches.rs:56).
- Names: top crates name the scenario (`fifty_tags_fit_and_a_fifty_first_does_nothing`, RE/tests/tags.rs:107); bottom crates use `validation`, `check_order`, `delete_batch`, and one name claims behavior the test does not check (LI/tests/batches.rs:126).
- Helper honesty: QE's `tag` and `send` helpers call `h.err` (QE/tests/tags.rs:16, QE/tests/max_message_bytes.rs:10), so a reader cannot tell from a call that it expects failure.
- Dead-letter moves ignoring MaxMessageBytes is covered by BO, KI, LI, PI, QA, QE, RE, VO, ZA and missed by BA, PA, VU.

## Patterns seen in most crates
- All crates reuse the BASE harness and compare exact JSON values or error codes; no test compares against values from the code under test, and all cases are deterministic.
- Nearly all fill a queue with 10000 batches to test LimitExceeded inside a batch (QA and LI skip it).
- Several byte-count comments are wrong (BA/tests/max_message_bytes.rs:20, PI/tests/max_message_bytes.rs:34): the data is right, the comment is not.
- A few `["ok"] == false` checks with no error code remain in most crates (BO, LI, BASE binary tests).
