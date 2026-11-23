# p7f / types

## Method

I read BASE `src/` in full. For each of the 21 attempts I ran the same script: a `diff -U1` against
BASE for every changed `.rs` file under `src/` (with new-file line numbers), plus the full text of
every new `src/` file. Every crate's `attributes.rs` diff got its own side-by-side check, and those
diffs are nearly identical: all add `max_message_bytes` as `u64` / `Option<u64>` parsed in range,
except WA, which uses `usize`. Tests were not read, because this dimension is about the types in
`src/`. In every crate I asked the same five questions:
(1) are batch entries parsed into a typed value before the per-entry handler runs, or handed on as a
JSON map / `Params`; (2) are tag keys and values held as their own types or as bare
`BTreeMap<String, String>`; (3) is validated data turned back into untyped JSON (re-validated, or
rebuilt into a command map); (4) does `Id` stay a `&str` (it does everywhere); (5) is the new code
consistent with BASE's typed pieces (`ErrorCode`, `Attributes`).

BASE itself is mixed. Error codes, message states and receive steps are enums, and the dead-letter
pair is `Option<DeadLetter>`. But commands are dispatched by matching strings
(BASE/src/broker.rs:72), message ids and receipt handles are bare `u64` (BASE/src/queue.rs:16, :28),
and every handler reads parameters by name from a JSON-backed `Params`. The attempts that kept this
pattern and added no new types score the same as BASE. An attempt scores higher for each new parse
into a typed value, and lower when it turns typed data back into untyped JSON.

## Scores

| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| WA | 4 | 1 | WA/src/tags.rs:18 (`TagKey`, `TagValue`, `Tags(BTreeMap<TagKey,TagValue>)`) | WA/src/broker/batch.rs:23 (entries still `(&str, Params)`) | Send args parsed into `SendArgs` (message_commands.rs:58), tag newtypes, `MaxMessageBytes` as `usize` so there is no cast |
| BA | 3 | 2 | BA/src/tags.rs:16 (`TagKey`/`TagValue` newtypes, `Tags` set) | BA/src/broker/batch_commands.rs:71 (handler takes raw `Params`, send args not parsed) | Tags typed; batch entries passed on untyped |
| RO | 3 | 2 | RO/src/tags.rs:16 (`TagKey`, `TagValue`, `Tags` map of both) | RO/src/broker/batch_commands.rs:75 (handler takes raw `Params`) | Same shape as BA |
| PO | 3 | 2 | PO/src/tags.rs:37 (`Tag { key: TagKey, value }` keeps the pair together) | PO/src/tags.rs:66 (value stays a bare `String`) | Key typed, value not; batch untyped |
| BU | 3 | 2 | BU/src/broker/message_commands.rs:56 (`SendParams` parsed once, passed inward) | BU/src/queue.rs:85 (`tags: BTreeMap<String,String>`) | Send typed, tags bare strings |
| FU | 3 | 2 | FU/src/broker/message_commands.rs:56 (`SendArgs`) | FU/src/queue.rs:75 (tags bare strings) | Send typed, tags bare strings |
| ME | 3 | 2 | ME/src/broker/message_commands.rs:55 (`SendArgs`; `IdCounter` wrapper at :84) | ME/src/broker/message_commands.rs:177 (`issued: u64` bare handle counter) | Send typed, tags bare strings (queue.rs:75) |
| NA | 3 | 2 | NA/src/broker/message_commands.rs:56 (`SendArgs`) | NA/src/queue.rs:76 (tags bare strings) | Send typed, tags bare strings |
| NE | 3 | 2 | NE/src/broker/message_commands.rs:55 (`SendArgs`) | NE/src/queue.rs:74 (tags bare strings) | Send typed, tags bare strings |
| NO | 3 | 2 | NO/src/broker/message_commands.rs:55 (`SendParams`) | NO/src/queue.rs:76 (tags bare strings) | Send typed, tags bare strings |
| RE | 3 | 2 | RE/src/broker/message_commands.rs:55 (`SendArgs`) | RE/src/queue.rs:84 (tags bare strings) | Send typed, tags bare strings |
| SO | 3 | 2 | SO/src/broker/message_commands.rs:56 (`Send` struct) | SO/src/broker/message_commands.rs:88 (`next_message_id: &mut u64`, `issued: u64` at :275) | Send typed, counters and tags bare |
| BASE | 2 | 13 | BASE/src/attributes.rs:33 (`Option<DeadLetter>` pairs DLQ + count) | BASE/src/broker.rs:72 (commands matched as strings) | Mixed starting point |
| CA | 2 | 13 | CA/src/tags.rs:16 (a `Tags` type exists) | CA/src/tags.rs:35 (`set_all(&Map<String,Value>)`: raw JSON past the boundary, re-read with a silent `if let Some` at :47) | Values checked, then the raw JSON is stored |
| LU | 2 | 13 | LU/src/naming.rs:17 (`is_tag_key` rule in one place) | LU/src/broker/batch_commands.rs:20 (`EntryObject` is an alias of a raw `&Map`) | Base pattern, no new types |
| MI | 2 | 13 | MI/src/broker/batch_commands.rs:15 (`Entry { id, params }`) | MI/src/queue.rs:75 (tags bare strings) | Base pattern, no new types |
| VI | 2 | 13 | VI/src/broker/batch_commands.rs:16 (`Entry` struct) | VI/src/broker/batch_commands.rs:84 (handler signature takes `&Map<String,Value>`) | Raw JSON map is the handler interface |
| WI | 2 | 13 | WI/src/broker/batch_commands.rs:15 (`Entry` struct) | WI/src/queue.rs:77 (tags bare strings) | Base pattern, no new types |
| KI | 1 | 19 | KI/src/broker/batch_commands.rs:16 (`Entry` struct) | KI/src/broker/batch_commands.rs:94 (clones entry map, inserts `QueueName` JSON, re-parses as a command) | Checked data rebuilt into an untyped command |
| LA | 1 | 19 | LA/src/broker/queue_commands.rs:17 (`tag_key` returns the checked key) | LA/src/broker/batch_commands.rs:70 (clone + insert `QueueName` into a JSON map) | Checked data rebuilt into an untyped command |
| SU | 1 | 19 | SU/src/broker/batch_commands.rs:61 (ids collected once) | SU/src/broker/batch_commands.rs:71 (re-derives the map with `.cloned().expect("checked")`, inserts `QueueName`) | Checks thrown away, then re-asserted with `expect` |
| ZI | 1 | 19 | ZI/src/broker/tag_commands.rs:37 (value matched by `Value::String` variant) | ZI/src/broker/batch_commands.rs:82 (clone + insert `QueueName` into a JSON map) | Checked data rebuilt into an untyped command |

## What separates the top from the bottom

- **Parse once into a struct vs pass `Params` on.** Eleven crates turn an entry's send parameters
  into a `SendArgs`-like struct before the queue is touched (e.g. NE/src/broker/message_commands.rs:65),
  so the single command and the batch entry share one typed value. Everyone else hands a JSON-backed
  `Params` to the shared handler, which reads `Body` by name again (e.g. VI/src/broker/message_commands.rs:64).
- **Tag newtypes.** WA, BA, RO and PO give tag keys (and, except PO, values) their own types and a
  `Tags` collection keyed by them (WA/src/tags.rs:50). The other 17 store `BTreeMap<String, String>`
  on the queue and check keys with a `check_key(&str) -> Result<()>` whose result is thrown away
  (e.g. MI/src/broker/tag_commands.rs:18).
- **Turning checked data back into JSON.** KI, LA, SU and ZI clone each entry's JSON object, insert
  `QueueName` and run the single command on that map (ZI/src/broker/batch_commands.rs:82). The
  checked `Id` and queue name go back into a `Map<String, Value>`. SU adds an `expect("checked")`
  to recover what it had already checked (SU/src/broker/batch_commands.rs:71).
- **Raw JSON stored past the check.** CA checks tag values, then hands the raw `&Map<String, Value>`
  to `Tags::set_all`, which has to re-read each value and silently skips a non-string
  (CA/src/tags.rs:47).
- WA alone types `MaxMessageBytes` as `usize`, converted once with an invariant `expect`
  (WA/src/attributes.rs:131). Every other crate compares a `usize` length against a `u64` through an
  `as` cast or `try_from` at each send.

## Patterns seen in most crates

- Every crate adds the four batch error codes to the existing `ErrorCode` enum, and none compares
  codes as strings.
- Every crate keeps the batch entry `Id` as a borrowed `&str` and checks distinctness on `&str`. No
  crate gives it a type, which is acceptable since it is only echoed back.
- All 21 dispatch the five new commands with BASE's string `match`, so command names stay a
  string-compared closed set throughout.
- `MaxMessageBytes` is added the same way everywhere (an `Option<u64>` in the update, a plain field
  on `Attributes`), so it does not separate the crates.
- No crate keeps the queue name as anything but `&str`/`String`, and no crate introduces a
  message-id or receipt-handle type. The new code inherits BASE's bare `u64` handles (ME and SO pass
  the handle counter around as `issued: u64`).
