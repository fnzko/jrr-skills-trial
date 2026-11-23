# p7g / tests

## Method

Same procedure for all 21 crates, no `cargo` run:

- Counted `#[test]` functions in `tests/` and in `src/`, and listed every test name to judge naming and spec coverage.
- Grepped every test file (integration files plus `src` files with `#[test]`) for weak assertions (`is_ok()`, `is_err()`, `is_some()`, `is_none()`, `is_empty()`, `contains(`), for `if`/`match` blocks in test code (assertions behind branches), and for wall-clock use (`Instant::now`, `elapsed`).
- Counted keyword hits for the main spec behaviors (dedup window, dead-letter, retention, ChangeMessageVisibility, handle Invalid vs Expired, LimitExceeded, QueueAlreadyExists, UnsupportedOperation, ListQueues/Prefix, invalid UTF-8, Purge, DeleteQueue, SequenceNumber, Tick, SetQueueAttributes, 1209601 bound, `r-01`/`R-1` handles).
- Counted assertions that compare a whole response (`json!({"ok": true, ...})` or an exact line) against ones that pick out a single field (`r[i]["X"]`).
- Read the same test in every crate (the FIFO dedup-window test), plus the shared harness and every flagged conditional or substring line.

## Scores

| code | score | rank | best evidence | worst evidence | reason |
|---|---|---|---|---|---|
| BU | 4 | 1 | BU/tests/send.rs:116 | BU/tests/send.rs:205 | whole responses with exact ids, plus case tables of exact codes; picks a field only now and then |
| FO | 4 | 1 | FO/tests/miniq/messages.rs:285 | FO/tests/miniq/queues.rs:58 | exact full responses; the per-case `match` asserts on both arms |
| JU | 4 | 1 | JU/tests/messages.rs:314 | JU/tests/messages.rs:535 | 42 whole-response asserts against 1 field pick |
| LO | 4 | 1 | LO/src/broker/tests.rs:339 | LO/tests/binary.rs:36 | every step checks the exact response line; the binary test is thin |
| RA | 4 | 1 | RA/tests/protocol.rs:393 | RA/tests/protocol.rs:34 | each test checks the whole session's expected responses; only the free-text message is dropped |
| SU | 4 | 1 | SU/tests/protocol.rs:288 | SU/tests/protocol.rs:464 | whole-session `assert_responses`; a few field picks |
| XI | 4 | 1 | XI/tests/protocol.rs:406 | XI/tests/protocol.rs:16 | whole-session exact asserts, widest coverage (handles, DLQ, retention bounds); one `a \|\| b` framing assert |
| ZU | 4 | 1 | ZU/tests/it/messages.rs:142 | ZU/tests/it/receive.rs:519 | whole sessions, 59 tests covering nearly every numbered rule |
| BO | 3 | 9 | BO/tests/protocol.rs:591 | BO/tests/protocol.rs:604 | exact values, but often only `["MessageId"]` instead of the whole response |
| FA | 3 | 9 | FA/src/queue.rs:634 | FA/tests/protocol.rs:193 | sound unit tests at queue level; only 14 protocol tests, so SetQueueAttributes rule order is thin |
| KE | 3 | 9 | KE/tests/it/behavior.rs:314 | KE/tests/it/behavior.rs:76 | full responses in the core flows; about half the asserts pick a field |
| LE | 3 | 9 | LE/tests/miniq/messages.rs:212 | LE/tests/miniq/messages.rs:38 | 73 tests, wide coverage; many single-field checks |
| MA | 3 | 9 | MA/tests/protocol.rs:407 | MA/tests/protocol.rs:61 | exact but fewest tests (29); field picks outnumber whole responses |
| MO | 3 | 9 | MO/tests/scenarios.rs:320 | MO/tests/scenarios.rs:163 | step-by-step exact asserts; 28 tests, many single-field |
| QE | 3 | 9 | QE/tests/receive.rs:50 | QE/tests/send.rs:185 | 45 behavior tests with good coverage, but `s.ok(..)["MessageId"]` picks dominate (4 whole vs 39 picks) |
| RO | 3 | 9 | RO/tests/behavior/messages.rs:248 | RO/tests/behavior/queues.rs:104 | full receive payloads asserted; attribute checks done field by field |
| VA | 3 | 9 | VA/tests/messages.rs:79 | VA/tests/protocol.rs:187 | exact full receive payloads; one substring check on the binary path |
| VU | 3 | 9 | VU/tests/protocol.rs:407 | VU/tests/protocol.rs:210 | exact error-code slices, but a few large multi-rule sessions mix concerns |
| ZE | 3 | 9 | ZE/tests/protocol.rs:465 | ZE/tests/protocol.rs:200 | exact responses; attributes checked field by field |
| NA | 2 | 20 | NA/tests/fifo_and_dead_letter.rs:48 | NA/tests/binary.rs:29 | no whole-response asserts at all (0 vs 33 picks); binary framing checked by substring |
| WI | 2 | 20 | WI/tests/messages.rs:414 | WI/tests/messages.rs:594 | wall-clock timing assert (flaky by design), substring checks on the binary path, picks dominate |

## What separates the top from the bottom

- Whole-session expected output against single fields. RA/tests/protocol.rs:393 and XI/tests/protocol.rs:406 state the full response list for a session, so an extra or missing member fails. NA/tests/fifo_and_dead_letter.rs:48 and QE/tests/send.rs:185 check only `["MessageId"]`, so a wrong `SequenceNumber` or an extra member passes.
- Exact framing checks at the binary boundary. LO/tests/binary.rs:39 compares exact lines, but NA/tests/binary.rs:29-33 and WI/tests/binary.rs:29-32 use `contains("InvalidRequest")` / `contains("\"Now\":1")`.
- Determinism. WI/tests/messages.rs:594 asserts on `Instant::elapsed() < 20s`. The other crates test the 100000 limit without a clock.
- Breadth of rule-order tests. XI, ZU, BU, LE and FO walk each command's numbered rules. MA and MO (28-29 tests) and FA (14 protocol tests) cover fewer SetQueueAttributes and handle-ordering cases.
- A harness that gets rid of noise without hiding anything. RA/tests/protocol.rs:34 and ZU/tests/it/common.rs:35 strip only the free-text `message`, and the rest of the response is still compared exactly.

## Patterns seen in most crates

- Test names describe the scenario everywhere (for example `a_moved_message_that_is_too_old_for_the_dead_letter_queue_is_removed_on_arrival`). No crate has generic names.
- No crate hides an assertion behind a branch that might not run. The `if`/`match` blocks found build inputs (`.fifo` names) or assert on every arm.
- Expected ids (`m-1`, `r-2`, `SequenceNumber`) are written as literals, not taken from the code under test.
- Unit tests for validators often use `assert_eq!(X::parse(s).is_ok(), valid)` or `is_some()`/`is_none()` (for example BO/src/types.rs:267). These are exact for a boolean validity rule, but they do not check which error variant comes back.
- Coverage of the spec's hard cases is nearly universal: dedup window not extended, dedup outliving purge, DLQ arrival past retention, FIFO group blocking, handle Invalid vs Expired, and the 100000 limit.
