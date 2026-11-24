# p14g / tests

## Method

Same procedure for all 12 crates, read against SPEC.md only:

1. Counted `#[test]` functions in `tests/` and `src/`. Listed every test name to check scenario naming and spec coverage.
2. Ran the same greps on every crate's tests: `is_ok/is_err/is_some/is_none/is_empty` inside `assert!`, `contains/starts_with` inside `assert!`, `if`/`if let` in test bodies, wall-clock use (`Instant`, `elapsed`), `matches!`. Then a fixed checklist of spec tokens (100000, 1000 queues, `r-01`, `R-1`, `30.0`, `3e1`, `null`, invalid UTF-8, each error code, DLQ, retention, 299/300 dedup window, SequenceNumber, Prefix, Counts, FirstReceivedAt, ChangeMessageVisibility, PurgeQueue, DeleteQueue, 1024/1025 bytes, `.fifo`).
3. Read every crate's test helper module and binary/process test in full. Read the main integration file(s) in full, or skimmed them with an assertion-line filter when they ran past about 400 lines. Read the `#[cfg(test)]` modules of crates with many unit tests (CU, PE, TA) and grepped the weak-assertion patterns in all the others.
4. Checked whether each crate runs the real binary (`CARGO_BIN_EXE`), and whether a few less obvious spec behaviors are covered: a new dead-letter configuration applying from the next receive, a delayed FIFO head blocking its group, and a queue recreated under a deleted name.

## Scores

| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| DO | 4 | 1 | DO/tests/protocol.rs:124 (named table of 34 precedence cases, each with its exact code) | DO/tests/protocol.rs:944 (`all(ok)` over 1000 replies, which hides which one failed) | Exact whole-response comparisons everywhere, and `session` asserts one reply per line; the widest coverage |
| PA | 4 | 1 | PA/tests/receive.rs:51 (whole delivery object with SentAt 5 and FirstReceivedAt 12) | PA/tests/handles.rs:11 (`send` helper drops its reply without checking it) | 62 scenario-named tests, exact values, covers handle expiry via DLQ, retention, purge and another queue |
| SE | 4 | 1 | SE/tests/miniq/messages.rs:38 (full redelivered message: r-2, count 2, FirstReceivedAt 3) | SE/tests/miniq/messages.rs:103 (asserts only bodies after the delay) | Exact replies, precedence sequences asserted step by step, binary tested; only a few body-only checks |
| TA | 4 | 1 | TA/tests/protocol.rs:40 (`assert_session` compares every reply of a session, message stripped) | TA/tests/protocol.rs:76 (concatenates sliced lines into one string to compare) | Whole sessions compared exactly; 25 focused unit tests; stderr checked empty in the binary test |
| CI | 3 | 5 | CI/tests/protocol.rs:301 (full redelivery objects with new handles r-3 and r-4) | CI/tests/protocol.rs:34 (`tick` helper ignores its reply; setup sends and receives also go unchecked) | Exact and table-driven, but setup steps are never checked, so a failing step goes unnoticed |
| CU | 3 | 5 | CU/tests/protocol.rs:106 (code sequence for the CreateQueue rules) | CU/src/types.rs:283 (`is_err()` with no error variant) | Good unit and end-to-end split, but several `is_ok/is_err` checks and an `if` that computes the expected code (request.rs:336) |
| HU | 3 | 5 | HU/tests/receive.rs:50 (whole delivery object) | HU/tests/receive.rs:271 (test named for the DLQ's new SequenceNumber never asserts it) | Exact values and checked helpers, but no test runs the binary, and one test name over-promises |
| PE | 3 | 5 | PE/tests/protocol.rs:198 (every reply of a SetQueueAttributes session, with exact codes) | PE/src/types.rs:243 (`Body::parse(..).is_err()` with no variant) | The strictest whole-session comparisons, but fewer scenarios: no binary test, no "new DLQ config from next receive", no delayed FIFO head |
| XI | 3 | 5 | XI/tests/protocol.rs:537 (exact MessageId and SequenceNumber after the dedup window) | XI/tests/protocol.rs:472 (`unwrap_or("ok")` turns any reply without a code into "ok") | Broad and mostly exact; the first protocol test compares only the `ok` flags (line 83) |
| ZO | 3 | 5 | ZO/tests/receive.rs:46 (exact two-message receive) | ZO/tests/receive.rs:121 (asserts only the array length for the delayed case) | Exact replies and table-driven validation; `..._stays_fast` (receive.rs:374) measures no speed |
| HO | 2 | 11 | HO/tests/queues.rs:150 (full Attributes object after a set) | HO/tests/queues.rs:161 (says the DLQ config cannot be removed, but asserts on queue `b`, not `a`) | Checked helpers, but the binary test matches on substrings (binary.rs:29-33), and one comment contradicts its assertion (fifo_and_dead_letter.rs:37) |
| NO | 2 | 11 | NO/tests/messages.rs:53 (whole delivery object) | NO/tests/messages.rs:594 (wall-clock `elapsed() < 20` assertion, flaky under load) | Good scenario tests let down by a timing assertion, substring checks in every binary test, and `is_err` unit checks with no variant (src/request.rs:185) |

## What separates the top from the bottom

- **Whole replies versus fragments.** DO, TA, PE and PA compare the entire response JSON, often for every line of a session (TA/tests/protocol.rs:40, PE/tests/protocol.rs:25). HO and NO check the binary's output with `contains("InvalidRequest")` (HO/tests/binary.rs:29, NO/tests/binary.rs:29). Those checks would pass on a wrong shape or extra members.
- **Determinism.** NO asserts on wall-clock time (NO/tests/messages.rs:594). Every other crate either does no timing check, or relies on the test simply finishing (CI/tests/protocol.rs:674), or tests behavior only while the name promises speed (ZO/tests/receive.rs:374).
- **Assertions that test what they claim.** HO/tests/queues.rs:161 inspects the wrong queue. HU/tests/receive.rs:248 is named for a new sequence number it never checks. The top crates have no such gap that I found.
- **Checked setup.** HU, NO, HO and SE either use `ok()` helpers that assert success or compare every reply. CI (CI/tests/protocol.rs:34) and PA's handle helper (PA/tests/handles.rs:11) let setup failures pass silently.
- **Breadth of edge behavior.** DO and PA cover the less obvious rules: a delayed FIFO head blocking its group (DO/tests/protocol.rs:618), a DLQ config applying from the next receive, handles of dead-lettered or expired messages (PA/tests/handles.rs:160). PE covers fewer of these and has no process-level test.

## Patterns seen in most crates

- Every crate tests through the line protocol with `serde_json::json!` expectations, and reads errors by their exact code rather than by message text. Most strip or ignore `message` correctly.
- Nearly all cover the hard parts of the spec: handle spelling (`r-01`, `R-1`, `r-1 `), integer literals (`30.0`, `3e1`), byte-length bodies with `é`, the 299/300 dedup window, the 100000-message and 1000-queue limits, and lowering retention.
- Test names describe scenarios in almost all crates (for example `a_dead_lettered_message_older_than_the_target_retention_is_removed_on_arrival`).
- The common weaknesses are small: unit tests use `is_ok()/is_err()` without naming the variant (CU, NO, PE, XI), binary tests fall back to `contains`/`starts_with` for the error line, and some setup replies go unchecked. "A failed command changes no state" is rarely tested directly.
