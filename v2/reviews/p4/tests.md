# p4 / tests
## Method
For each of the 12 crates I ran the same steps. No crate has a `BASE` directory, so every crate was judged in full.
1. Counted `#[test]` functions, `assert*` calls, and the weak-assertion lines (`is_ok/is_err/is_some/is_none/contains/is_empty/starts_with` inside an assert), and read every hit.
2. Listed every test name.
3. Grepped test code for `if/match/for/while` to find assertions behind branches or loops, and for `Instant/elapsed/sleep` to find nondeterminism.
4. Read the shared test harness (`tests/common`, `support`, or the helper block at the top of the main test file) in full, plus about the first 100 lines of the largest integration test file.
5. Read one unit-test module from `src`, picked from the weak-assertion hits when there were any.
6. Grepped tests and `src` for spec terms (each error code, 100000, 1000, `30.0/3e1`, `r-01/R-1`, 300/299, Prefix, RetentionSeconds, MaxReceiveCount, SequenceNumber, ContentBasedDeduplication) as a coverage check.
Coverage came out about the same in every crate: every crate exercises all ten codes, both limits, the dedup window, FIFO group blocking, dead-letter moves and retention. So the scores mostly reflect how precise the assertions are and how much of the setup is checked.

## Scores
| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| PA | 4 | 1 | PA/tests/messages.rs:23 | PA/src/types.rs:350 | Each scenario compares the full response transcript to spec-derived JSON; names are long and specific; only a stray bare `is_err`. |
| XI | 4 | 2 | XI/tests/messages.rs:78 | XI/tests/messages.rs:60 | Whole responses compared, message stripped after a type check, setup asserted via `ok()`; the `tick` helper ignores its reply. |
| WE | 4 | 3 | WE/tests/service/common.rs:38 | WE/tests/service/messages.rs:42 | `check()` asserts every response of a session, setup included; test names like `send_parameter_rules` are vague and bundle many rules. |
| MU | 3 | 4 | MU/tests/integration/support.rs:31 | MU/tests/integration/dead_letters.rs:73 | Errors checked as an exact object and values are exact; `send`/`run` setup replies are often dropped unchecked. |
| ZU | 3 | 5 | ZU/tests/common/mod.rs:27 | ZU/tests/messages.rs:110 | Exact whole responses and an error helper that checks the response shape; a few setup calls (SetQueueAttributes) are left unchecked. |
| QE | 3 | 6 | QE/tests/protocol.rs:44 | QE/tests/protocol.rs:110 | Full-object comparisons with expected values documented as spec-derived; some scenarios check one field of a delivery. |
| KE | 3 | 7 | KE/tests/messages.rs:31 | KE/src/request.rs:483 | Helpers assert success or failure plus the message type; exact values throughout; a few unit tests stop at `is_ok()`. |
| FU | 3 | 8 | FU/tests/commands.rs:103 | FU/tests/binary.rs:58 | Good exact checks of the default attributes and of rule order; a wall-clock bound makes one test timing-dependent; `is_ok()` in unit tests. |
| QI | 3 | 9 | QI/tests/protocol.rs:39 | QI/tests/protocol.rs:188 | Whole-value comparisons, but setup commands inside table loops are executed without any check; one test pins behaviour at the clock limit, which the spec leaves partly unspecified. |
| SO | 3 | 10 | SO/src/params.rs:166 | SO/src/params.rs:162 | Many focused tests with exact error variants; deliveries are checked field by field, the `fail` helper ignores `message`, and one check is a substring test. |
| TA | 3 | 11 | TA/tests/messages.rs:589 | TA/tests/handles.rs:91 | Broad, well-named, table-driven coverage; transcripts are indexed by position and checked one field at a time, and loops over responses have no length check. |
| NI | 2 | 12 | NI/src/attributes.rs:210 | NI/src/attributes.rs:199 | Range tests stop at bare `is_ok/is_err`; deliveries checked field by field; responses built with the crate's own `success/failure`; a wall-clock assertion at broker_messages.rs:471. |

## What separates the top from the bottom
- Whole-response comparison against literal JSON. PA (messages.rs:23), WE (common.rs:38) and XI (messages.rs:78) compare complete replies, so a missing or extra member fails the test. NI (broker_messages.rs:88-98) and SO (receive.rs:96-103) assert one member per line and prove absence with `get(..).is_none()`, so an extra member they did not think of passes.
- Every setup step checked. WE's `check()` pairs every request with its expected reply, and XI and ZU assert inside `create`. In QI (protocol.rs:188), MU (dead_letters.rs:73) and TA (`last()` in common/mod.rs:30), setup replies are dropped, so a test can end up exercising a state it did not intend.
- Error variants rather than truthiness. KE, SO and FU mostly assert `unwrap_err().code == ErrorCode::X`. NI (attributes.rs:198-206) checks range bounds with bare `is_err()`, which passes on any error code.
- Determinism. FU (binary.rs:58) and NI (broker_messages.rs:471) bound wall-clock time inside the normal test run, so a slow CI machine can fail them. The others assert the limit only by its result: the 100000th message is accepted and the next one gets `LimitExceeded`.
- The error message is normalized, not ignored. PA (support/mod.rs:32), WE and XI assert that `message` is a string and then remove it so the rest of the reply can be compared exactly. SO's `fail` (common/mod.rs:22) never looks at the message.

## Patterns seen in most crates
- Every crate drives the real line-handling path (`handle_line`/`serve`) for its integration tests, plus at least one test that spawns the binary or exercises framing (blank lines, invalid UTF-8, a final line without `\n`).
- Every crate tests the receipt-handle edge cases from the spec (`r-01`, `R-1`, `r-1 `), the 299/300 dedup boundary, and the 100000-message limit through the protocol.
- Expected values are literals taken from the spec almost everywhere; no crate computes expected values by calling the code under test. NI comes closest, by building its responses with the crate's own `success/failure`.
- Table-driven cases (`for (input, expected) in cases`) carry the input in the assert message, so a failure names its case. The only branch inside a test is a table split into valid and invalid cases, and both arms run.
- The common weakness is setup commands whose replies nobody checks, and per-field checks of deliveries where a whole-object comparison would have been stricter.
