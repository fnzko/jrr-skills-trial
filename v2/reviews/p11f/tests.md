# p11f / tests

## Method

For every attempt I ran `diff -ruN BASE/tests <CODE>/tests` and read the whole diff (new files `batches.rs`/`batch.rs`, `tags.rs`, `max_message_bytes.rs`, and the added hunks in `queues.rs`/`messages.rs`). I also listed every `#[cfg(test)]` module under `src/` and compared it with BASE; only the new `src/tags.rs` unit tests in BE, ME, QA, SA and WI differ, and I read those. Inherited BASE tests were not scored for attempts. BASE was scored on its whole test suite (all of `tests/` plus the `request.rs` and `naming.rs` unit tests), read the same way. Each diff was checked against the same list from SPEC-feature.md: the four batch error codes and their order (empty, too many, invalid id, distinct; after the queue lookup), "a failed request does nothing", per-entry handling seeing earlier entries (FIFO dedup, ids, the message limit, used handles), all-failed still a success, both result arrays in order, tag rules (key/value bounds in bytes, step 3 before lookup, 50 limit with replacement, all-or-nothing), tags vs attributes/purge/delete, MaxMessageBytes range, default, rule 4, rule order, existing messages and dead-letter moves. Then the same assertion-quality checks: exact values vs `is_ok`/`len`, loops that may not run, misleading names.

## Scores

| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| BE | 4 | 1 | BE/tests/batches.rs:62 | BE/tests/max_message_bytes.rs:45 | Exact (Id, code) lists, every spec rule covered; only a trailing unasserted create |
| CA | 4 | 1 | CA/tests/batches.rs:215 | CA/tests/batches.rs:236 | Full coverage with exact values; the delete-batch assertion is built in a roundabout way |
| CU | 4 | 1 | CU/tests/batches.rs:161 | CU/tests/batches.rs:116 | Foreign handle, wrong type, all failed paths asserted exactly; one very long rule test |
| FA | 4 | 1 | FA/tests/batches.rs:154 | FA/tests/tags.rs:139 | Shape rules run for both batch commands; tag check-order test repeats two asserts verbatim |
| HU | 4 | 1 | HU/tests/batches.rs:129 | HU/tests/max_message_bytes.rs:52 | Limit test checks the count before the edge; one test name does not match what it checks |
| NO | 4 | 1 | NO/tests/batches.rs:192 | NO/tests/batches.rs:259 | Every request rule asserted for both commands; expected messages are copied from the response |
| QA | 4 | 1 | QA/src/tags.rs:112 | QA/tests/batches.rs:325 | Exact integration and unit tests incl. all-or-nothing tag limit; a few single-field checks |
| QE | 4 | 1 | QE/tests/batches.rs:227 | QE/tests/batches.rs:298 | Broad exact coverage; the 99,990-message fill loop never checks its results |
| RI | 4 | 1 | RI/tests/batches.rs:217 | RI/tests/batches.rs:132 | Tests the limit edge and freed slot exactly; one comment contradicts its assertion (m-3); no dead-letter test |
| RU | 4 | 1 | RU/tests/batch.rs:325 | RU/tests/batch.rs:136 | Partial-fill limit test with exact ids; no dead-letter test, one single-field check |
| SA | 3 | 11 | SA/tests/max_message_bytes.rs:64 | SA/tests/batches.rs:91 | Good coverage, but several checks are only lengths or `is_ok`/`is_err` (SA/src/tags.rs:96) |
| ME | 3 | 11 | ME/tests/batches.rs:40 | ME/src/tags.rs:114 | Batches are exact; MaxMessageBytes thin (no dead-letter test, existing messages only by count), unit tests use `is_ok`/`is_err` |
| WI | 3 | 11 | WI/tests/batches.rs:96 | WI/tests/batches.rs:82 | Shape rules for both commands; test named for the message limit does not test it, which is left uncovered |
| BASE | 3 | 11 | BASE/tests/messages.rs:42 | BASE/tests/binary.rs:29 | Whole-response equality throughout; binary tests use substrings, request unit tests `is_ok`/`is_err` (BASE/src/request.rs:183) |
| GU | 2 | 15 | GU/tests/tags.rs:74 | GU/tests/batches.rs:56 | Assertions sit inside a loop over `Failed` with no count check; `send`/`tag` helpers that assert errors; length-only batch byte test |
| CI | 2 | 15 | CI/tests/batches.rs:103 | CI/tests/batches.rs:126 | Test named `message_limit_...` checks the id counter, so the limit is untested; `["ok"], false` with no code (CI/tests/max_message_bytes.rs:59); unformatted |

## What separates the top from the bottom

- Exact per-entry results. The top crates collect `Failed` into `(Id, code)` pairs and compare the whole list (BE/tests/batches.rs:62, CU/tests/batches.rs:161, RI/tests/batches.rs:217). GU loops over `Failed` and asserts each code, which passes on an empty array (GU/tests/batches.rs:56).
- Honest names. CI/tests/batches.rs:126 and WI/tests/batches.rs:82 promise a message-limit test and do not test the limit, so the spec rule that entries see the 100,000 message limit goes unchecked in both. Most others fill the queue and check the boundary (HU/tests/batches.rs:129, RU/tests/batch.rs:325).
- Coverage of both batch commands. FA, NO and WI run the numbered rules for `DeleteMessageBatch` too (FA/tests/batches.rs:154); CI only checks shape errors for `SendMessageBatch`.
- Variant over boolean. Weaker crates check `["ok"], false` (CI/tests/max_message_bytes.rs:59) or a length (SA/tests/batches.rs:91) where an error code was available.
- Edge precision. RI and RU fill to exactly N slots below the limit and check which entry is the first to fail (RU/tests/batch.rs:325); HU checks the visible count before the last batch.

## Patterns seen in most crates

- Nearly all reuse BASE's `Harness` (`ok`/`err` return exact codes), so most assertions compare whole JSON values or exact error codes, and expected values are hand-written (`m-1`, `SequenceNumber`), not read back from the code under test.
- Every crate updated the `create_uses_defaults` expectation in `queues.rs` with `MaxMessageBytes: 1024`.
- Almost all cover: check order (missing/invalid parameter, lookup, then numbered rules), "too many wins over bad ids", "bad id wins over duplicates", FIFO dedup within one batch, used handles in one delete batch, tag replace-at-limit all-or-nothing, tags surviving purge and dying with delete, UTF-8 byte counting for both tag values and bodies.
- Common small gaps: the dead-letter exemption for MaxMessageBytes is missing in ME, RI and RU; several crates include one long "kitchen sink" rule test; unit tests added in `src/tags.rs` often fall back to `is_ok`/`is_err` (ME, SA, WI).
- Tests are deterministic (Tick-driven clock, no randomness), and each builds its own `Harness`, so they are independent.
