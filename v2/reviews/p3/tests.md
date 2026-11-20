# p3 / tests
## Method
For every crate (BASE included) I counted `#[test]` items in `tests/` and `src/`, ran `diff -rq BASE/tests <crate>/tests`,
extracted every `#[cfg(test)]` module from `src/` and diffed it against BASE's module of the same file (new modules
were read in full). I read BASE's whole test suite once (tests/*.rs, request.rs and naming.rs test modules) against
SPEC.md for coverage, since 16 of 18 attempts left `tests/` byte-identical. Each attempt is judged on what it changed
relative to that baseline. I also checked every crate still exports `Broker::new`/`handle_line`, which the
integration harness uses. No cargo run; same greps and diffs for every crate.

Baseline lapses in BASE: tests/binary.rs checks responses only by substring (`contains("\"Now\":1")`, lines 29-32,
40, 53); request.rs `range_is_inclusive` uses `is_ok()`/`is_err()`; `shape_errors` ends with `is_ok()`;
queues.rs:257 is a boolean `is_none() && is_none()`; messages.rs:594 is a wall-clock timing assertion. Everything
else asserts exact error codes, exact response objects and exact ids, has scenario names, and covers the spec well
(error order, dedup window, DLQ moves, retention, FIFO group blocking, limits).

## Scores
| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| ME | 4 | 1 | ME/tests/binary.rs:38 (parsed JSON == exact object) | ME/tests/queues.rs:257 (boolean is_none) | Fixed the substring binary tests and the is_ok unit tests; asserts typed `command` |
| JI | 4 | 2 | JI/src/request.rs:195 (exact parsed command) | JI/tests/binary.rs:31 (starts_with prefix, key-order dependent) | Fixed binary tests, queues.rs:257 and the unit-test is_ok calls; one prefix match remains |
| CU | 3 | 3 | CU/src/ids.rs:76 (table of handle spellings -> exact Option) | CU/tests/binary.rs:29 (substring check) | New exact, well-named unit tests for ids, time, QueueName; unit lapses fixed, binary.rs untouched |
| PI | 3 | 4 | PI/src/request.rs:209 (exact command) | PI/tests/binary.rs:29 (substring check) | Fixed both unit-test is_ok lapses only |
| TE | 3 | 4 | TE/src/request.rs:210 (exact command) | TE/tests/binary.rs:29 (substring check) | Same fix as PI |
| ZA | 3 | 4 | ZA/src/request.rs:192 (exact Some(1)) | ZA/tests/binary.rs:29 (substring check) | Same fix as PI, plus asserts exact message wording |
| FE | 3 | 7 | FE/src/request.rs:192 (exact Some(1) and error code) | FE/src/request.rs:206 (is_ok) | Fixed range test only |
| SI | 2 | 8 | SI/src/request.rs:254 (exact Error value) | SI/src/request.rs:261 (is_ok) | Added exact tests for require order, unknown command, handles; left every base lapse |
| QO | 2 | 9 | QO/src/ids.rs:73 (handle spellings -> None) | QO/src/ids.rs:69 (boolean chain assertion) | Added id unit tests; left base lapses and added a boolean one |
| BASE | 2 | 10 | BASE/tests/messages.rs:53 (exact message object) | BASE/tests/binary.rs:29 (substring check) | Strong integration suite, substring and is_ok lapses |
| FI | 2 | 10 | FI/tests/messages.rs:53 | FI/tests/binary.rs:29 | Tests identical to BASE |
| FO | 2 | 10 | FO/tests/messages.rs:53 | FO/tests/binary.rs:29 | Tests identical to BASE |
| HO | 2 | 10 | HO/tests/messages.rs:53 | HO/tests/binary.rs:29 | Tests identical to BASE |
| NU | 2 | 10 | NU/tests/messages.rs:53 | NU/tests/binary.rs:29 | Tests identical to BASE |
| PO | 2 | 10 | PO/tests/messages.rs:53 | PO/tests/binary.rs:29 | Tests identical to BASE |
| SU | 2 | 10 | SU/tests/messages.rs:53 | SU/tests/binary.rs:29 | Tests identical to BASE |
| VA | 2 | 10 | VA/tests/messages.rs:53 | VA/tests/binary.rs:29 | Tests identical to BASE |
| VO | 2 | 10 | VO/tests/messages.rs:53 | VO/tests/binary.rs:29 | Tests identical to BASE |
| WO | 2 | 10 | WO/tests/messages.rs:53 | WO/tests/binary.rs:29 | Tests identical to BASE |

## What separates the top from the bottom
- Only ME and JI touched the integration tests, replacing `contains(...)` checks on whole response lines with exact
  comparisons (ME/tests/binary.rs:38-41 parses and compares objects; JI compares raw strings, with one prefix match
  at JI/tests/binary.rs:31). JI also split the boolean at JI/tests/queues.rs:257-258 into two `assert_eq`.
- Mid-tier attempts (PI, TE, ZA, FE, CU) fixed the `is_ok()/is_err()` range test into exact values and error
  variants (e.g. PI/src/request.rs:194), and most also replaced the trailing `is_ok()` with an exact parsed command.
- CU, SI and QO added new unit tests for the types they introduced (ids, time, QueueName) with table-driven exact
  expectations (CU/src/ids.rs:76, QO/src/ids.rs:73, SI/src/request.rs:254), but SI and QO kept every base lapse and
  QO added `assert!(first < second && second < next)` (QO/src/ids.rs:69).
- Eleven attempts made no change to any test, so they inherit BASE's score exactly.
- ME and ZA now assert exact `message` text (ME/src/request.rs:201), which the spec leaves free; exact, but it
  couples the test to wording.

## Patterns seen in most crates
- All crates share BASE's strong integration suite: exact error codes per validation step, exact response objects,
  explicit id/handle numbering, deterministic virtual time, independent `Harness::new()` per test, and scenario names.
- Nearly all keep the wall-clock assertion at tests/messages.rs:594 (`elapsed() < 20s`), the one non-deterministic
  check, and the substring-only checks in tests/binary.rs.
- No attempt removed or weakened a test, and none added integration coverage for behavior the refactor touched.
