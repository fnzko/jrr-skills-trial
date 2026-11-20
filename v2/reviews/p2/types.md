# p2 / types

## Method

- BASE: read every file under `src/` in full (lib, error, naming, request, attributes, queue, broker, broker/queue_commands, broker/message_commands) and scored it as a whole.
- Each attempt: ran `diff -ruN BASE/src <CODE>/src` and read every added or changed line, new files included. Tests were not read, since this dimension is about production types.
- I asked the same questions of every attempt. Are batch entries parsed into a typed value, or passed inward as raw JSON / `Params`? Do tag keys and values get their own types? Is the batch `Id` kept together with its entry? How is `MaxMessageBytes` typed and compared? Is anything rebuilt as `serde_json::Value` past the boundary?
- Located evidence with the same greps in every crate (`struct SendArgs|Entry|Send`, `pub tags:`, `insert("QueueName"`, `expect("checked`, `as u64 >`, `fn parse_entries|check_key`).
- Inherited code is not scored for attempts. Every attempt inherits the untyped `Members` result map and the bare-`u64` ids and handles, so those do not separate them.

## Scores

| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| NO | 4 | 1 | NO/src/tags.rs:18 (`TagKey`, `TagValue`, `Tags(BTreeMap<TagKey,TagValue>)`) | NO/src/broker/batch.rs:99 (batch outcome built straight into `json!`) | Tags parsed into domain types, batch entries parsed into `SendArgs`, `MaxMessageBytes` converted to `usize` with a checked `try_from` (NO/src/attributes.rs:131) |
| CU | 3 | 2 | CU/src/broker/message_commands.rs:55 (`SendArgs` parsed once, passed inward) | CU/src/broker/tag_commands.rs:30 (tags are `BTreeMap<String,String>`, key rule only a check) | Typed send args and (id, Params) pairs; tags stay bare strings; `IdCounter` wraps a `&mut u64` rather than an id type |
| FI | 3 | 2 | FI/src/broker/message_commands.rs:56 (`SendArgs`) | FI/src/queue.rs:75 (`BTreeMap<String,String>` tags) | Same shape as CU: typed send path, string tags, `&mut u64` counter passed raw |
| PA | 3 | 2 | PA/src/broker/batch_commands.rs:16 (`Entry { id, members }` keeps the id with its entry) | PA/src/queue.rs:75 (string tags) | Named entry struct and `SendArgs`; entry members still a raw `Map` re-wrapped later; tags untyped |
| VI | 3 | 2 | VI/src/tags.rs:16 (`TagKey`/`TagValue`/`Tags` with `set_all`) | VI/src/broker/message_commands.rs:64 (`send_to_queue(name, params: Params)`: the entry goes in unparsed) | Strong tag types and a checked length conversion (VI/src/broker/message_commands.rs:80); batch entries reach the handler as untyped `Params` |
| VU | 3 | 2 | VU/src/broker/message_commands.rs:61 (`SendEntry` typed) | VU/src/broker/tag_commands.rs:29 (`BTreeMap<&str,&str>`, stored as `String,String`) | Typed send path; tag keys and values are bare strings |
| XA | 3 | 2 | XA/src/broker/message_commands.rs:56 (`SendArgs`, `Copy`) | XA/src/queue.rs:76 (string tags) | Typed send path; tags untyped |
| XO | 3 | 2 | XO/src/tags.rs:37 (`Tag { key: TagKey, value }`, `Tags(BTreeMap<TagKey,String>)`) | XO/src/broker/message_commands.rs:63 (`send_to(name, params: Params)` parses inside) | Typed tags (value left a `String`); batch path passes raw `Params` inward |
| BASE | 2 | 9 | BASE/src/error.rs:7 (closed `ErrorCode` enum; also `State`/`Step` enums, `DeadLetter` struct) | BASE/src/queue.rs:228 (`deliver(key: u64, now: u64, visibility_timeout: u64, handle: u64)`) | Good enums and `Option`, but arrival key, receipt handle and message id are all bare `u64`; `delayed_until: 0` and `invisible_until: 0` stand in for "not set" (BASE/src/broker/message_commands.rs:115); results are an untyped `Map<String,Value>` |
| PO | 2 | 9 | PO/src/broker/batch_commands.rs:16 (`(id, Params)` pairs validated once) | PO/src/broker/message_commands.rs:63 (`send_to(&str, Params)`: no typed args) | No raw-JSON rebuilding, but entries never become typed values; string tags |
| RO | 2 | 9 | RO/src/broker/batch_commands.rs:23 (id paired with entry) | RO/src/broker/message_commands.rs:64 (`send_to(&str, Params)`) | Same as PO; entries kept as `&Map<String,Value>` |
| WU | 2 | 9 | WU/src/broker/tag_commands.rs:18 (`check_key` returns the checked `&str`) | WU/src/broker/message_commands.rs:63 (`send_to(&str, Params)`) | Same as PO; string tags |
| ZU | 2 | 9 | ZU/src/broker/message_commands.rs:129 (`Send` typed args) | ZU/src/broker/message_commands.rs:306 (`entry.as_object().expect("checked by batch_ids")`: ids parsed apart from entries, entries re-derived) | Typed send, but validation throws away the object it checked and re-casts it later; string tags |
| JO | 1 | 14 | JO/src/broker/queue_commands.rs:27 (`tag_key` returns the checked key) | JO/src/broker/message_commands.rs:123 (clones entry JSON, inserts `"QueueName"`, re-dispatches) | Batch rebuilds a JSON command map and reparses it through the single handler; string tags |
| KE | 1 | 14 | KE/src/broker/tag_commands.rs:18 (key check in one place) | KE/src/broker/batch_commands.rs:71 (ids split from entries, `cloned().expect("checked")`, then `"QueueName"` injected at :72) | Worst batch path: validation result discarded, raw JSON cloned and mutated |
| PI | 1 | 14 | PI/src/broker/batch_commands.rs:20 (id kept with its entry) | PI/src/broker/batch_commands.rs:83 (entry `Map` cloned, `"QueueName"` inserted as `Value::String`) | Same JSON-synthesis approach as JO; string tags |

## What separates the top from the bottom

- Batch entries. The 3s parse each entry into a typed `SendArgs`/`SendEntry`/`Send` and pass that inward (CU/src/broker/message_commands.rs:55, VU/src/broker/message_commands.rs:61). JO, KE and PI clone the entry's `serde_json::Map`, write `"QueueName"` into it, and hand it back to the string-keyed handler (JO/src/broker/message_commands.rs:123, KE/src/broker/batch_commands.rs:72, PI/src/broker/batch_commands.rs:83).
- Tag domain types. Only NO, VI and XO give tags their own types (NO/src/tags.rs:18, VI/src/tags.rs:16, XO/src/tags.rs:21). The other twelve store `BTreeMap<String,String>` and treat the key rule as a passing check (e.g. FI/src/queue.rs:75).
- Keeping the entry Id with its entry. Most return `(id, Params)` from validation, and PA names the pair (PA/src/broker/batch_commands.rs:16). ZU and KE validate the ids into a separate `Vec<&str>` and later re-cast each entry with `expect` (ZU/src/broker/message_commands.rs:306, KE/src/broker/batch_commands.rs:71).
- Numeric conversion for `MaxMessageBytes`. NO stores `usize` via `try_from` (NO/src/attributes.rs:131) and VI uses `u64::try_from` (VI/src/broker/message_commands.rs:80). The other 13 cast `body.len() as u64` (e.g. XO/src/broker/message_commands.rs:78). The cast is harmless here but unchecked.
- Middle tier (PO, RO, WU, VI, XO). They pass `Params` inward to `send_to(&str, Params)`. There is no JSON mutation, but no typed value either, so the parse still happens inside the handler (PO/src/broker/message_commands.rs:63).

## Patterns seen in most crates

- All 15 add the four error codes as `ErrorCode` variants and never compare codes as strings. All 15 type `MaxMessageBytes` as an `Option<u64>` update plus a `u64` attribute, like the existing attributes.
- None introduces a type for the batch outcome. Every crate builds `Successful`/`Failed` directly as `serde_json::Value` (e.g. NO/src/broker/batch.rs:99), and every crate keeps BASE's untyped `Members` result map.
- No attempt gives the batch entry `Id` its own type. It stays a validated `&str` everywhere.
- None fixes BASE's bare-`u64` handle, id and arrival key. CU's `IdCounter(&mut u64)` (CU/src/broker/message_commands.rs:84) wraps the counter, not the id.
