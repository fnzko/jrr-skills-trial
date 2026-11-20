# p2 / overall

## Method

- Read SPEC.md (sections 1 to 3) and SPEC-feature.md in full, then read BASE/src in full (every file).
- For each of the 15 attempts, produced `diff -ruN BASE/src <CODE>/src` and read every added and
  removed line of it (new modules included), in the same order: attributes, batch, message commands,
  tags, broker dispatch, error, queue, request. Inherited code was not scored.
- For tests, the same mechanical pass on each `diff -ruN BASE/tests <CODE>/tests`: list of test
  function names (to check coverage of: entries see earlier effects, all-failed is still a success,
  batch rule order, MaxMessageBytes before rule 1, 50-tag limit, tags survive purge but not delete),
  and counts of `assert_eq`, `is_ok/is_err` and `contains(` assertions. No attempt got a closer read
  of its tests than another. Tests weigh little here: all 15 are similar (17 to 42 tests, exact
  `assert_eq`, no `is_ok`/substring assertions).
- Question asked of each delta: how much would I need to learn before trusting a change to batches,
  tags or the size limit, and can a change to `SendMessage`/`DeleteMessage` silently diverge from
  the batch path?

## Scores

| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| BASE | 4 | 1 | BASE/src/queue.rs:194 (`plan_receive` plans without mutating) | BASE/src/broker.rs:86 (`expect("required")` after `require`, a convention you must learn) | Small, well-split, typed attributes and errors; check order is explicit. |
| PO | 4 | 1 | PO/src/broker/message_commands.rs:63 (`send_to` = SendMessage from step 2 on, batch reuses it unchanged) | PO/src/broker/batch_commands.rs:38 (comment patching over `.ok().flatten()` swallowing the type error) | Smallest faithful delta: one `run_batch`, single and batch share one code path. |
| RO | 4 | 1 | RO/src/broker/batch_commands.rs:66 (`run_batch` takes `fn(&mut Self,&str,Params)`) | RO/src/request.rs:133 (returns `&Vec<Value>` instead of a slice) | Same shape as PO plus a shared `Error::to_json`. |
| WU | 4 | 1 | WU/src/broker.rs:78 (dispatch passes `Self::send_to` straight into `batch`) | WU/src/broker/batch_commands.rs:83 (error JSON shape rebuilt by hand, duplicating broker.rs) | Minimal split of the single commands, no new types. |
| JO | 4 | 1 | JO/src/broker/message_commands.rs:108 (`run_batch` calls the real single command per entry) | JO/src/broker/message_commands.rs:122 (clones each entry map to inject `QueueName`) | Batch cannot drift from SendMessage/DeleteMessage; but batches/tags grow existing files. |
| PI | 4 | 1 | PI/src/broker/batch_commands.rs:66 (`run_batch` reusing `send_message`/`delete_message`) | PI/src/broker/tag_commands.rs:40 (three-arm match where a filter suffices) | JO's approach in its own modules; clone justified by a comment. |
| VI | 4 | 1 | VI/src/tags.rs:57 (`Tags::set_all` owns the 50-tag invariant over typed `TagKey`/`TagValue`) | VI/src/broker/message_commands.rs:80 (`u64::try_from(len).map_or(true, ..)` for a usize that cannot overflow) | PO-style batch plus a typed tag module with its own unit test. |
| FI | 3 | 8 | FI/src/broker/batch_commands.rs:62 (`run_entries` shared result builder) | FI/src/broker/message_commands.rs:85 (whole `send_message` body moved to a free fn: 96 BASE lines rewritten) | Clean but large churn of inherited code to thread `&mut u64` around. |
| PA | 3 | 8 | PA/src/broker/batch_commands.rs:16 (`Entry` struct names id + members) | PA/src/broker/batch_commands.rs:105 (each handler re-does its own entry plumbing in a closure) | Readable, a bit more surface than needed. |
| NO | 3 | 8 | NO/src/tags.rs:54 (typed `Tags` with `count_after_set`) | NO/src/broker/batch.rs:64 (`require("Body")` repeated, while NO/src/broker/message_commands.rs:67 also maps a missing Body) | Typed tags; the "Body required" decision lives in two places. |
| VU | 3 | 8 | VU/src/broker/batch_commands.rs:75 (`run_batch` with closures over parsed `SendEntry`) | VU/src/naming.rs:16 (tag key limit moved into `naming`, apart from the other tag limits) | Solid; small scattering of tag constants and an identity `batch_error` (line 15). |
| XO | 3 | 8 | XO/src/tags.rs:70 (`Tags::set_all` keeps the limit check with the data) | XO/src/broker/message_commands.rs:62 (`send_to` relies on callers having required `Body`, stated only in a comment) | Good typing, one precondition held by comment rather than code. |
| KE | 3 | 8 | KE/src/broker.rs:78 (dispatch reuses `Self::send_message` for the batch) | KE/src/broker/batch_commands.rs:71 (re-reads entries with `cloned().expect("checked")`, unformatted line) | Right idea, one long function that validates twice. |
| XA | 3 | 8 | XA/src/broker/batch_commands.rs:58 (`batch_result` shared) | XA/src/broker/batch_commands.rs:78 and :98 (the batch prologue copied in both handlers) | Fine split of SendMessage; copied preamble invites drift. |
| CU | 2 | 15 | CU/src/broker/batch_commands.rs:63 (`batch_result` shared) | CU/src/broker/message_commands.rs:84 (`IdCounter` newtype with a single use, wrapping `&mut u64`) | Big rewrite of inherited send/delete, identity `batch_error` (line 14), quadratic dedup (line 52). |
| ZU | 2 | 15 | ZU/src/broker/message_commands.rs:53 (`batch_ids` keeps batch rules in one place) | ZU/src/broker/message_commands.rs:306 and :317 (entries re-parsed with `expect("checked by batch_ids")`, twice) | Batches crammed into message_commands.rs (362 lines), copied handler bodies, null tag values silently dropped (ZU/src/request.rs:147). |

## What separates the top from the bottom

- Whether batch entries go through the single command's own code. PO, RO, WU, VI split each single
  command at "step 2 on" (`send_to(name, params)`), and JO, PI call the single command itself
  (JO/src/broker/message_commands.rs:108); a later change to SendMessage then cannot miss the batch.
  CU and FI moved the whole SendMessage body into a free function threaded with counters
  (CU/src/broker/message_commands.rs:84), which is a larger inherited-code rewrite to trust.
- Validate once, carry the result. Top crates return `(id, Params)` from entry parsing; KE and ZU
  return only ids and re-open the entries with `expect` (KE/src/broker/batch_commands.rs:71,
  ZU/src/broker/message_commands.rs:306).
- One batch prologue. Most have one `run_batch`; XA and ZU copy the require/lookup/parse preamble
  into both handlers (XA/src/broker/batch_commands.rs:78 and :98).
- Placement. Separate `batch_commands.rs`/`tag_commands.rs` (or `tags.rs`) keep files at BASE's
  scale; ZU and, less so, JO grow message_commands.rs/queue_commands.rs with unrelated features.
- Typed tags (NO, VI, XO) move the 50-tag invariant next to the data (VI/src/tags.rs:57); they
  add a module but make the limit hard to bypass.

## Patterns seen in most crates

- Nearly all add `Params::array` plus a raw-members accessor and reuse `naming::is_identifier`
  for entry ids and tag keys: they extend BASE's vocabulary instead of inventing parallel parsing.
- Nearly all fold every `Id` failure into `InvalidBatchEntryId` with `.ok().flatten()`, which
  discards the typed error on purpose but reads as a swallowed `Result` (PO adds a comment, line 38).
- `body.len() as u64 > max_message_bytes` is the common size check; harmless, but a `usize`
  attribute (NO) or a shared constant type would avoid the cast.
- The `{"Id", "error"}` JSON is rebuilt by hand in about half the crates; the others add
  `Error::to_json` and reuse it in `handle_line`, removing a second copy of the error shape.
- Tests are uniformly exact-value integration tests through the line protocol and cover the spec's
  key behaviors; they do not separate the crates.
