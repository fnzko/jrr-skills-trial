# p7f / tests

## Method

For every crate I ran `diff -rq BASE/tests <CODE>/tests` and counted `#[test]` in new files and in
added lines, then read in full every test file that is new relative to `BASE` (`batches.rs` or
`batch.rs`, `tags.rs`, `max_message_bytes.rs` where present) and the `diff` hunks against `BASE` for
`queues.rs` and `messages.rs`. I grepped `src/` for `#[test]`; only RO adds unit tests (`src/tags.rs`),
which I read too. The rest of `src/` was not read. Each crate was checked against the same list of
SPEC-feature behaviors:
- batch rules 1-4 and their order (too many before a bad Id, a bad Id before duplicates)
- the earlier checks and the lookup coming before the batch rules
- a rejected batch doing nothing
- per-entry failures not stopping the others and using no ids
- FIFO dedup and sequence numbers seen across entries
- the 100000-message limit reached mid-batch
- DeleteMessageBatch handles used earlier in the batch, and handles from another queue
- an all-failed batch still being a success
- tag key and value rules, including the UTF-8 byte boundary of values
- the 50-tag limit with replaced keys, and a failed TagQueue changing nothing
- tags ignored by GetQueueAttributes and by CreateQueue rule 4, kept by Purge, dropped by Delete
- MaxMessageBytes range, default, and part in rule 4
- the size check counting UTF-8 bytes and coming before SendMessage rule 1
- lowering the limit keeping messages that are already queued
- dead-letter moves ignoring the limit

For assertion quality I looked at each test for these weak forms:
- `["ok"] == false` with no error code
- length-only checks
- `is_ok`/`is_err`
- loops whose assertions may not run
- expected values copied from the response
- names that do not match what the test does

BASE was scored on the same scale for this phase: it has no tests for any feature behavior.

## Scores

| code | score | rank | best evidence | worst evidence | reason |
|---|---|---|---|---|---|
| BA | 4 | 1 | BA/tests/batches.rs:55 | BA/tests/batches.rs:104 | Full coverage, failed entries compared as exact (Id, code) lists; a few index-only code checks |
| BASE | 1 | 22 | BASE/tests/messages.rs:41 | BASE/tests/queues.rs:27 | Its existing assertions are exact, but it has no test of any feature behavior |
| BU | 3 | 9 | BU/tests/batches.rs:163 | BU/tests/queues.rs:308 | Excellent exact tables and check order; no test that dead-letter moves ignore MaxMessageBytes |
| CA | 4 | 1 | CA/tests/batches.rs:87 | CA/tests/max_message_bytes.rs:121 | Checks even the member set of Failed entries; the dead-letter setup send is unasserted |
| FU | 3 | 9 | FU/tests/batches.rs:70 | FU/tests/tags.rs:78 | Good exact lists; no test of a tag value over the UTF-8 byte limit, no dead-letter test, no rejected-batch-does-nothing test |
| KI | 4 | 1 | KI/tests/batches.rs:27 | KI/tests/batches.rs:103 | Helper asserts the exact Failed shape; one expected list is built with an `if` inside a map |
| LA | 3 | 9 | LA/tests/batches.rs:62 | LA/tests/tags.rs:87 | Several `["ok"], false` with no code (tags.rs:87, max_message_bytes.rs:41,67); rule order (rule 2 before 3, rule 3 before 4) not tested |
| LU | 4 | 1 | LU/tests/batches.rs:65 | LU/tests/batches.rs:133 | Complete and exact; delete-batch codes compared without their Ids |
| ME | 4 | 1 | ME/tests/batches.rs:112 | ME/tests/max_message_bytes.rs:10 | Exact (Id, code) pairs everywhere, including both entries over the message limit; `send` returns `Result<(), String>` |
| MI | 2 | 19 | MI/tests/batches.rs:58 | MI/tests/batches.rs:198 | Test named for the message limit actually tests MaxMessageBytes, so the 100000 limit is never reached; several count-only checks (:75, :83, :160) |
| NA | 4 | 1 | NA/tests/batches.rs:164 | NA/tests/tags.rs:43 | Exact codes for both batch commands per case; no test that a 256-byte value made of two-byte characters is accepted |
| NE | 3 | 9 | NE/tests/batches.rs:33 | NE/tests/tags.rs:81 | `a_bad_entry_applies_nothing` ignores the TagQueue response; expected `message` values are copied from the response (batches.rs:42) |
| NO | 4 | 1 | NO/tests/batches.rs:154 | NO/tests/tags.rs:156 | Thorough table run over both commands; tags.rs:156-163 repeats two assertions verbatim |
| PO | 3 | 9 | PO/tests/batches.rs:189 | PO/tests/batches.rs:259 | Expected error messages copied from the response under test; wrong byte-count comment at max_message_bytes.rs:23 |
| RE | 4 | 1 | RE/tests/batches.rs:193 | RE/tests/batches.rs:143 | Most complete case tables, plus side-effect counts on two queues; FIFO failure codes listed without Ids |
| RO | 3 | 9 | RO/tests/batches.rs:73 | RO/tests/batches.rs:149 | Message-limit test checks only `Failed[1]` and leaves its fill loop unasserted; `is_ok`/`is_err` unit test (src/tags.rs:96) |
| SO | 3 | 9 | SO/tests/batches.rs:242 | SO/tests/batches.rs:92 | Strong request table; test name claims the message limit but covers only size; `send` returns the error code; variable `ten` holds one entry (:255) |
| SU | 2 | 19 | SU/tests/batches.rs:54 | SU/tests/batches.rs:126 | `message_limit_seen_by_later_entries` only checks the id counter, so the 100000 limit is untested; sparse batch check order; odd `Value2` alias |
| VI | 3 | 9 | VI/tests/batches.rs:106 | VI/tests/tags.rs:145 | Broad coverage; several `["ok"], false` with no code and length-only receive checks (batches.rs:117, 214, 243) |
| WA | 3 | 9 | WA/tests/batch.rs:350 | WA/tests/queues.rs:332 | Best mid-batch limit test (exactly 5 accepted and 5 failed); no test that dead-letter moves ignore the limit |
| WI | 3 | 9 | WI/tests/batches.rs:155 | WI/tests/tags.rs:159 | Exact mid-batch limit; neither CreateQueue-rule-4-ignores-tags nor the dead-letter rule is tested |
| ZI | 2 | 19 | ZI/tests/batches.rs:154 | ZI/tests/batches.rs:56 | Assertion inside a loop over Failed that may run zero times; count-only check (:129); `tag`/`send` helpers return error codes |

## What separates the top from the bottom

- Top crates collect `Failed` into an exact `(Id, code)` list and compare it in one assertion
  (BA/tests/batches.rs:55, ME/tests/batches.rs:112, KI/tests/batches.rs:27). Bottom crates check
  `Failed[0]` or `Failed[1]` alone, or a length (RO/tests/batches.rs:149, ZI/tests/batches.rs:129,
  MI/tests/batches.rs:75).
- Coverage of the 100000-message limit reached mid-batch: most crates fill the queue with 10000
  batches and assert exactly. MI (batches.rs:198) and SU (batches.rs:126) have a test whose name
  claims this but checks something else.
- Rule-order cases: the top crates test both "too many before a bad Id" and "a bad Id before
  duplicates", and run the table through both batch commands (RE/tests/batches.rs:193,
  NO/tests/batches.rs:154). LA and SU test only part of this.
- Weak checks: `["ok"], false` with no error code (LA/tests/tags.rs:87, VI/tests/tags.rs:145,
  SU/tests/max_message_bytes.rs:59), responses that are ignored (NE/tests/tags.rs:81), and
  assertions inside a loop that may run zero times (ZI/tests/batches.rs:56).
- Expected values copied from the output: NE (batches.rs:42) and PO (batches.rs:259) put the
  response's own `message` into the expected JSON. This is harmless for free-text messages but it
  is the one place the oracle comes from the code under test.
- Spec gaps that cost a point:
  - the dead-letter-ignores-MaxMessageBytes rule is untested in BU, FU, WA and WI
  - WI also leaves out "CreateQueue rule 4 ignores tags"
  - FU has no test of a tag value over the UTF-8 byte limit

## Patterns seen in most crates

- All 21 crates reuse the BASE `Harness` (`ok`/`err` assert `ok` and return the error code), so
  error codes are compared exactly almost everywhere and nothing depends on time or order across
  tests.
- Nearly every crate tests the UTF-8 byte boundary with `é` (two bytes) on both sides of the limit,
  and tests that the size rule comes before SendMessage rule 1, using a standard queue with a GroupId.
- Nearly all show "a failed TagQueue changes nothing" by mixing a valid member with an invalid one or
  with an over-limit one, then listing the tags.
- A common small weakness is `assert!(attrs["Attributes"].get("Tags").is_none())` instead of
  comparing the whole attribute object. CA and NO compare the full object (CA/tests/tags.rs:183).
- Many tests look up error messages only with `is_string()`. That fits the spec, but it means the
  `message` member is never pinned.
