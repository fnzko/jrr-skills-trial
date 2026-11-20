# p2 / tests

## Method

- BASE: read all of `tests/` (common, protocol, binary, queues, messages) plus the `#[cfg(test)]`
  modules in `src/request.rs` and `src/naming.rs`. Scored as a whole.
- Each attempt: `diff -rq BASE <CODE>` to list changed files. For every test file that is new I read
  it in full, and for every changed test file I read the `diff` hunks. I also checked whether any
  `#[cfg(test)]` module in `src/` changed (none did; the module text matched BASE in all 15). So
  each attempt is scored only on what it added or changed in tests, and inherited tests are ignored.
- The same checklist went through every crate: (a) exact values vs `is_ok`/count/`ok == true`/
  substring checks; (b) per-entry `Failed` checked as (Id, code) pairs vs a bare `Failed[i]` code;
  (c) assertions behind branches or loops that may not run; (d) expected values taken from the
  response under test; (e) test names that match what is asserted; (f) spec coverage: the four batch
  rules and their order, check order before the rules, per-entry effects (dedup, ids, message
  limit), failures that do not stop the others, all-failed still a success, DeleteMessageBatch incl.
  a handle from another queue, the 50-tag limit with replacements, key/value byte rules, tags vs
  attributes/purge/delete, MaxMessageBytes default/range/UTF-8 bytes/before rule 1/existing
  messages/dead-letter moves ignore it/CreateQueue rule 4.

## Scores

| code | score | rank | best evidence | worst evidence | reason |
|---|---|---|---|---|---|
| CU | 4 | 1 | CU/tests/batches.rs:63 | CU/tests/batches.rs:147 | Exact (Id, code) pairs whose helper also checks the member count. Full coverage incl. DLQ, foreign handle and limit inside a batch. Only a count check here and there |
| FI | 4 | 1 | FI/tests/batches.rs:286 | FI/tests/batches.rs:320 | Exact pairs everywhere, failure leaves state untouched, DLQ, foreign handle and `r-01`. One check counts members only |
| PO | 4 | 1 | PO/tests/batches.rs:229 | PO/tests/batches.rs:255 | Every batch rule runs against both commands. Message ids continue after the limit and a purge (m-100001). Full coverage. A few count-only checks |
| RO | 4 | 4 | RO/tests/batches.rs:130 | RO/tests/max_message_bytes.rs:14 | Exact pairs, DLQ, limit. Its send helper turns a missing code into the sentinel `"ok"`, and one test name promises a body-range check it never makes |
| NO | 3 | 5 | NO/tests/batch.rs:343 | NO/tests/batch.rs:129 | Exact 5/5 split at the message limit. Some `Failed[i]` codes are checked without the Id. No test that dead-letter moves ignore MaxMessageBytes |
| PA | 3 | 5 | PA/tests/batches.rs:304 | PA/tests/batches.rs:149 | Very thorough delete batch. Content-dedup test checks only `Successful[1]`, `Visible` only in places, no DLQ-ignore test |
| VU | 3 | 7 | VU/tests/batches.rs:273 | VU/tests/batches.rs:99 | Full coverage, exact pairs. The all-failed case and the foreign handle are checked by count or by index only |
| BASE | 3 | 8 | BASE/tests/messages.rs:53 | BASE/tests/binary.rs:29 | Exact message objects and full error-code checks across the spec. binary.rs checks stdout with `contains` substrings |
| VI | 3 | 9 | VI/tests/batches.rs:73 | VI/tests/batches.rs:149 | Small tests with names that match what they assert, full coverage incl. DLQ. Limit and all-failed checked by index or count, no Id |
| WU | 3 | 10 | WU/tests/batches.rs:183 | WU/tests/messages.rs:649 | Exact split at the limit. Shown only as a count that existing messages survive a lower limit. `impl Harness` added in a test file. Some codes checked without the Id |
| JO | 3 | 11 | JO/tests/batches.rs:67 | JO/tests/batches.rs:83 | Exact pairs in the main cases, but several `Failed[0]` codes have no Id. Response at :156 unchecked. No DLQ-ignore test |
| XA | 2 | 12 | XA/tests/batches.rs:137 | XA/tests/batches.rs:47 | Expected `message` copied from the response under test. An `if code == "ok"` branch inside the assertion loop (:178). Codes without Ids (:78). No DLQ-ignore test |
| XO | 2 | 13 | XO/tests/max_message_bytes.rs:19 | XO/tests/batches.rs:259 | Expected messages again copied from the response. FIFO and limit failures checked by index without Ids. Byte-count comment is wrong (`max_message_bytes.rs:23`) |
| ZU | 2 | 14 | ZU/tests/messages.rs:700 | ZU/tests/messages.rs:826 | Asserts only `Failed + Successful == 10`. Batch and tag tests mixed into messages.rs/queues.rs. Byte-count comment wrong (:610). No DLQ-ignore test |
| KE | 2 | 15 | KE/tests/batches.rs:116 | KE/tests/batches.rs:126 | `message_limit_seen_by_later_entries` checks only the id counter, so the message limit in a batch is untested. `["ok"] == false` without a code (tags.rs:48, max_message_bytes.rs:59). `Failed[1]` code only |
| PI | 2 | 16 | PI/tests/batches.rs:154 | PI/tests/batches.rs:56 | A per-failure loop with no length check passes when `Failed` is empty. MaxMessageBytes in a batch checked by count (:129). The `tag` helper's name hides that it expects failure |

## What separates the top from the bottom

- Per-entry results: CU, FI, PO, RO, NO and VU collect `Failed` into ordered (Id, code) pairs and
  compare them whole (CU/tests/batches.rs:63, FI/tests/batches.rs:286). PI, KE, XO and ZU check
  `Failed[0]`/`Failed[1]` codes one at a time without the Id (PI/tests/batches.rs:172,
  KE/tests/batches.rs:32, XO/tests/batches.rs:138, ZU/tests/messages.rs:907).
- Expected values that come from the code under test: XA/tests/batches.rs:47 and
  XO/tests/batches.rs:259 fill the expected `message` from the response under test. The others
  check `message.is_string()` on its own (CU/tests/batches.rs:20).
- Assertions that can pass vacuously: PI/tests/batches.rs:56 loops over `Failed` with no length
  check. ZU/tests/messages.rs:826 asserts only a sum. KE/tests/batches.rs:126 has a name that claims
  a behavior it does not test.
- The message limit inside a batch: NO/tests/batch.rs:343, WU/tests/batches.rs:183 and
  XA/tests/batches.rs:127 leave room for 5 or 3 entries and check the exact split. KE never tests it.
  Most others fill the queue and check one entry.
- Coverage of "dead-letter moves ignore MaxMessageBytes": present in CU, FI, PO, RO, VI, VU, WU, XO,
  KE, PI. Missing in NO, PA, JO, XA, ZU.
- Foreign-queue handles and `r-01`-style handles in DeleteMessageBatch: tested by CU, FI, PA, PO,
  XA, XO, VU. Not tested by JO, KE, PI, RO, VI, WU, ZU.

## Patterns seen in most crates

- Every attempt kept BASE's `Harness` and its `err()` helper, so step-level errors are compared as
  exact codes almost everywhere. No attempt used `is_ok()` or substring checks in its new tests.
- Nearly all of them test the rule order (too many beats a bad Id, a bad Id beats duplicates, lookup
  before the numbered rules), that a rejected batch sends nothing, the UTF-8 byte edges with `é`
  (128 vs 129 two-byte characters for a tag value, `éé` vs `ééa` for a body), and the 50-tag limit
  with a mix of replaced and new keys that changes nothing.
- The common lapse is a lone count or index check (`as_array().len()`, `Failed[i]["error"]["code"]`)
  in a test that is otherwise exact. It usually sits in the ten-entry and 80-character Id cases or
  in the all-failed case.
- All attempts run the 100000-message fill through batches in every limit test (deterministic but
  slow). No attempt adds unit tests in `src/`.
- No attempt tests a bad `Id` in a DeleteMessageBatch entry beyond a missing `Id`, except where the
  shared rule table runs against both commands (PO, XA, XO, ZU).
