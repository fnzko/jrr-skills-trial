# p1 / tests
## Method
Same procedure for all 15 crates, with no cargo runs. (1) Located every file containing `#[test]` (integration tests under `tests/` and `#[cfg(test)]` modules under `src/`), counted tests, and counted these assertion forms in test code only: `assert_eq!`, bare `assert!`, `.is_ok/.is_err/.is_some/.is_none()`, `.contains(`, `is_empty()`, `if/match/if let` in test bodies, and wall-clock use (`Instant::now`/`elapsed`). (2) Listed every test name. (3) Read each crate's shared helper (`tests/common`, or the helper block at the top of the single test file) to see whether it compares whole responses or picks fields and whether it checks the error shape. (4) Counted whole-response expectations (`json!({"ok"...` / raw `{"ok"...` strings) against single-field picks (`["Messages"][0]["Body"]` etc., `bodies(...)`). (5) Grepped for spec behaviors (100000 limit, 1000 queues, retention, dedup window 299/300, exact handle strings `r-01`/`R-1`, `ChangeMessageVisibility`, `PurgeQueue`, null, `30.0`/exponent, invalid UTF-8, a spawned binary via `CARGO_BIN_EXE`, FIFO message objects with `GroupId`/`SequenceNumber`). (6) Read one receive/round-trip test in each crate plus every hit from step 1 that looked weak.

## Scores
| code | score | rank | best evidence | worst evidence | reason |
|---|---|---|---|---|---|
| HE | 4 | 1 | HE/src/broker/tests.rs:297 | HE/src/broker/tests.rs:10 | Every scenario is a script that checks each response as an exact string. Failures are checked by code only, and the rendered error shape is exercised only in binary.rs |
| QO | 4 | 1 | QO/tests/protocol.rs:43 | QO/src/types.rs:343 | Whole-response vectors with only the message text normalized, and the widest spec coverage. Its unit tests on value types use `is_err()`/`is_ok()` |
| FI | 4 | 1 | FI/tests/protocol.rs:78 | FI/src/protocol.rs:437 | 98 tests, including a check of error precedence and many whole-object expectations. Two parse tests stop at `is_ok()` |
| BI | 3 | 4 | BI/tests/fifo.rs:32 | BI/tests/common/mod.rs:11 | Exact FIFO message objects throughout. Only the in-process `run` is tested; no test spawns the binary or checks its exit code |
| CE | 3 | 4 | CE/tests/receive.rs:46 | CE/tests/receive.rs:145 | Good exact receive objects and a broad error-order suite. Redelivery is checked one field at a time |
| FU | 3 | 4 | FU/tests/it/behavior.rs:314 | FU/tests/it/behavior.rs:438 | Exact values and typed message builders. Only 40 tests, with thin coverage of ChangeMessageVisibility and PurgeQueue |
| LI | 3 | 4 | LI/tests/messages.rs:31 | LI/tests/messages.rs:63 | Whole delivery objects, no `is_ok()` and no substring checks. Several tests pick single fields |
| TA | 3 | 4 | TA/tests/binary.rs:65 | TA/src/request.rs:506 | The only test of flush-before-next-read, plus a precise dedup window. Errors are compared by code, line by line, and one unit test stops at `is_ok()` |
| TO | 3 | 4 | TO/tests/messages.rs:14 | TO/tests/process.rs:34 | Exact whole responses in the session tests. The process test checks error lines by substring |
| VU | 3 | 4 | VU/tests/messages.rs:20 | VU/tests/protocol.rs:40 | `script` compares every response whole. Few tests (36), a substring check, exact-handle strings not tested, and the binary test pins member order |
| WU | 3 | 4 | WU/tests/receive.rs:40 | WU/tests/receive.rs:80 | 76 tests; `assert_error` checks both code and message. Many single-field and `bodies` picks |
| ZE | 3 | 4 | ZE/tests/dead_letter.rs:181 | ZE/tests/dead_letter.rs:63 | Strong dead-letter coverage with exact outcome vectors. Receive counts are checked one field at a time |
| BO | 2 | 13 | BO/tests/receive.rs:43 | BO/tests/receive.rs:58 | Mostly code-only and single-field checks (4 whole-response expectations). No binary test, and FIFO message objects are barely asserted |
| PO | 2 | 13 | PO/tests/messages.rs:42 | PO/tests/messages.rs:594 | Exact round trip, but the load test uses a wall-clock bound, the binary test uses substrings, and unit tests use `is_ok()`/`is_err()` |
| MI | 1 | 15 | MI/tests/fifo_and_dead_letter.rs:48 | MI/tests/binary.rs:29 | No whole message object is ever compared (field picks, counts as `[u64;3]`), so extra or missing members go unnoticed. The binary test is all `contains` |

## What separates the top from the bottom
- Comparing whole responses. HE (HE/src/broker/tests.rs:16 `play`), VU (VU/tests/common/mod.rs `script`) and QO (QO/tests/protocol.rs:32 `assert_responses`) compare every response in full, so the "exactly these members" rule of the spec is enforced. MI never builds a full expected message (MI/tests/messages.rs:72-80 checks fields one by one) and BO has only 4 whole-response expectations.
- Substring and `is_ok()` assertions. MI/tests/binary.rs:29-33, PO/tests/binary.rs:29-32, TO/tests/process.rs:34 and VU/tests/protocol.rs:40 check JSON lines with `contains`. QO/src/types.rs:343-432 and PO/src/request.rs:183-195 stop at `is_ok()`/`is_err()` without the variant. The top crates use these at most once or twice.
- Process-level protocol. BI and BO never spawn the binary, so exit status 0 and stdout-only output go untested. TA/tests/binary.rs:65 is the only crate that checks each response arrives before the next line is written.
- Determinism. PO/tests/messages.rs:585-594 bounds a 100000-send fill with `Instant::now().elapsed() < 20s`, a timing assertion that can fail on a slow CI runner. The other crates test the limit only by its outcome.
- Breadth. FI (98 tests), WU (76), TO (73), ZE (65) and QO cover nearly every numbered rule, including the dedup window boundary at 299/300. VU, HE and FU have fewer tests and depend on long scripts. HE's scripts are exhaustive, while VU and FU leave gaps (exact handle strings, PurgeQueue).

## Patterns seen in most crates
- Test names describe the scenario in every crate (for example `a_message_too_old_for_the_dead_letter_queue_is_removed_on_arrival`). No crate uses names like `test1`.
- Expected values are literals from the spec, never computed by the code under test. Every case builds a fresh broker, so tests are independent. Only PO depends on wall time.
- Almost no assertion sits behind a branch that may not run. The few `if` lines in test code build inputs (FI/tests/messages.rs:467, TO/tests/fifo.rs:207) or normalize messages (QO).
- Most crates compare errors by code alone. They check that `message` is a string (CE, PO, WU) or ignore it, which the spec allows.
- The common lapse is reading single fields (`["Messages"][0]["ReceiveCount"]`) or `bodies(...)` for follow-up deliveries after one full-object check. Every crate except HE, VU and QO does this.
