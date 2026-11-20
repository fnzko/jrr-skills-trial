# p6g / tests
## Method
Same procedure for all 18 crates (no BASE directory exists):
- Listed every `#[test]` function name in `tests/` and in every `src/` file with a `#[cfg(test)]` module (plus NE's `src/broker/tests.rs`), and mapped the names against SPEC sections 1-6.
- Read each crate's test driver/helper module and the same three kinds of test in full: the dead-letter move, receipt-handle invalid/expired, and the protocol/framing (blank line, unterminated line, invalid UTF-8) tests.
- Ran identical greps over every crate: `assert.*is_ok()/is_err()`, `assert.*contains(`, `assert.*is_empty()`, `assert!(matches!`, `if`/`match` inside test bodies, `Instant`/`elapsed`, crate constants used as expected values, and `CARGO_BIN_EXE` (is the real binary and its exit code tested).
- Counted tests and assertion macros per crate as a rough coverage signal, not as a score by itself.
No `cargo` was run; whether the tests pass was not checked.

## Scores
| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| DE | 4 | 1 | DE/tests/messages.rs:571 | DE/src/protocol.rs:437 | 98 tests, unit and integration layers, every rule order and handle case covered with exact codes. Only lapse is `is_ok()` for valid bodies |
| HU | 4 | 2 | HU/tests/receive.rs:45 | HU/tests/receive.rs:123 | Whole delivery objects compared, handle lifecycle split into precise scenarios, binary tested; `is_empty` on empty receives is the weakest form |
| VE | 4 | 3 | VE/tests/miniq.rs:947 | VE/tests/miniq.rs:869 | Exact full-object checks, wide coverage including binary framing; a wall-clock bound in the limit test |
| TU | 4 | 4 | TU/tests/messages.rs:157 | TU/tests/messages.rs:643 | Drives the real binary end to end, compares ordered outcome lists and exact handles; 20 s wall-clock bounds |
| CE | 4 | 5 | CE/tests/it/behavior.rs:32 | CE/tests/it/behavior.rs:439 | Compares whole responses built from named constructors, checks "failed command changes no state"; a few checks look only at `Body` |
| CA | 3 | 6 | CA/tests/protocol.rs:664 | CA/src/parse.rs:330 | Broad spec coverage with exact values; unit test asserts `is_ok()` and many checks pick single fields |
| KO | 3 | 7 | KO/tests/messages.rs:221 | KO/tests/common/mod.rs:10 | Exhaustive handle and rule-order tables with exact codes, but the binary and its exit code are never run |
| JU | 3 | 8 | JU/tests/protocol.rs:42 | JU/src/types.rs:343 | Integration tests compare entire response lists; unit tests use about 17 `is_ok`/`is_err` asserts without checking the error |
| JI | 3 | 9 | JI/tests/receive.rs:43 | JI/tests/protocol.rs:28 | Exact full-detail checks and good coverage; no binary test, and the newline check counts `\n` bytes |
| NE | 3 | 10 | NE/src/broker/tests.rs:432 | NE/src/broker/tests.rs:8 | Every step compared to an exact line or typed `ErrorCode`, but the tests bypass the line loop (one binary test) and compare JSON text whose member order the spec leaves free |
| NO | 3 | 11 | NO/tests/protocol.rs:398 | NO/tests/protocol.rs:33 | Long ordered outcome lists and a binary test; the `outcome` helper lumps every success into "ok" |
| NI | 3 | 12 | NI/tests/protocol.rs:491 | NI/src/types.rs:243 | Normalised whole-response comparison; fewer scenarios (37 tests), no binary test, `is_err` units |
| SO | 3 | 13 | SO/tests/messages.rs:127 | SO/tests/binary.rs:29 | Precise handle and dead-letter scenarios; the binary test checks substrings only |
| LA | 3 | 14 | LA/tests/scenarios.rs:354 | LA/tests/scenarios.rs:561 | Good rule-order tables; dead-letter checks pick fields one by one, no binary test |
| TA | 3 | 15 | TA/tests/protocol.rs:54 | TA/src/queue.rs:756 | Good queue unit tests, but only 13 integration tests; the limit test loops to the crate's own `MAX_MESSAGES`, and there are `is_ok`/`is_err` units |
| RU | 3 | 16 | RU/tests/protocol.rs:219 | RU/tests/protocol.rs:413 | Exact codes for handle rules; partial `Counts["Visible"]` checks, 36 tests, no binary test |
| LI | 2 | 17 | LI/tests/messages.rs:218 | LI/tests/binary.rs:29 | Good handle tables, but framing is checked by `contains`, range units by `is_ok`/`is_err`, and moved messages field by field |
| QA | 2 | 18 | QA/tests/protocol.rs:271 | QA/tests/cli.rs:33 | Fewest tests (27). Large mixed scenarios leave gaps (moves not counting toward MaxMessages, FIFO dead-letter sequence, new dead-letter config), and the CLI checks substrings |

## What separates the top from the bottom
- **Whole responses compared, not single fields.** The top crates assert entire messages or response lists (HU/tests/receive.rs:45, CE/tests/it/behavior.rs:32, JU/tests/protocol.rs:42). Weaker ones probe single members (RU/tests/protocol.rs:413, LA/tests/scenarios.rs:557), which would miss an extra or wrong member.
- **Exact framing checks.** CE/tests/it/binary.rs:5 and NE/tests/binary.rs:18 compare stdout byte for byte. LI/tests/binary.rs:29, SO/tests/binary.rs:29 and QA/tests/cli.rs:33 accept any line that merely contains "InvalidRequest".
- **One behavior per test, named for it.** DE and HU split the handle rules into separate scenarios (DE/tests/messages.rs:557-620, HU/tests/handles.rs). QA packs send, receive, delay, expiry and invalid handles into one test (QA/tests/protocol.rs:271), so a failure points to no single rule.
- **Coverage depth.** DE (98 tests), HU (62) and TU (58) cover nearly every numbered rule, including subtle ones (DE/tests/messages.rs:522, a full FIFO queue still answers duplicates). QA (27) and RU (36) skip several dead-letter rules.
- **The real process.** TU drives every test through the binary (TU/tests/common/mod.rs:20). KO, JI, LA, NI and RU never check the exit code or real stdin/stdout.
- **Expected values written in the test.** Most crates write literal ids such as `m-1` and `r-3`. TA/src/queue.rs:756 loops to the crate's own constant, so a wrong limit would still pass.

## Patterns seen in most crates
- Nearly all crates use a small driver that runs lines through `run`/`handle_line` and returns parsed JSON, plus an `error_code` helper that first asserts `ok == false`. So code checks are exact and cannot pass silently on a success.
- Test names are descriptive scenario sentences in every crate. No crate has opaque names like `test1`.
- Rule-order behavior is commonly tested as a table of inputs against an ordered list of expected codes (CA, CE, DE, KO, LA, NO, TA, TU).
- Unit tests of validators often fall back to `is_ok()`/`is_err()` or `assert_eq!(x.is_ok(), valid)` without naming the error variant (JU, NI, LI, TA, CA, DE, TU, NE, LA, NO).
- No crate hides assertions behind a branch that may not run. The only `if`s in tests map outcomes or pick fixture names, and both arms assert (KO/tests/queues.rs:53).
- The limit tests that time themselves use a generous wall-clock bound (LI, NO, TU, VE), because the spec asks for speed. That is the only nondeterminism seen.
