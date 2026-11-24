# p11f / types

## Method
- BASE: read every file under `src/` in full and scored the whole crate.
- Each attempt: ran `diff -ru BASE/src <CODE>/src` and read every new `src/` file in full, the same way for all 15. Tests were not used for this dimension.
- For each crate I checked the same points: (1) how tag keys and values are represented, both in storage (`Queue::tags`) and between parse and apply; (2) whether batch entries and their `Id` are parsed into a typed value or passed on as raw `Params` or JSON; (3) whether the single `SendMessage` parameters are parsed into a struct that batch entries reuse; (4) how `MaxMessageBytes` is typed and compared; (5) whether JSON is rebuilt and passed inward.

## Scores
| code | score | rank | best evidence | worst evidence | reason |
|---|---|---|---|---|---|
| CA | 4 | 1 | CA/src/broker/message_commands.rs:56 (`SendEntry` parsed once, shared by batch) | CA/src/broker/batch_commands.rs:22 (entry `Id` kept as bare `&str` in a tuple) | Typed send arguments, `TagKey`/`TagValue`/`Tags` throughout. |
| QE | 4 | 1 | QE/src/tags.rs:66 (`Tags(BTreeMap<TagKey, TagValue>)`) | QE/src/broker/batch_commands.rs:20 (entry `Id` as `&str` tuple) | Typed `SendArgs`, typed key and value parsed at the boundary. |
| RU | 4 | 1 | RU/src/broker/message_commands.rs:58 (`SendArgs` reused by batch) | RU/src/broker/batch.rs:23 (entry `Id` as `&str` tuple) | Same as CA/QE; `max_message_bytes: usize` also avoids a cast. |
| BE | 3 | 4 | BE/src/broker/batch_commands.rs:17 (`EntryId` newtype) | BE/src/tags.rs:66 (tag value is a bare `String`) | Only crate that types the entry id, plus `TagKey`/`TagUpdate`; entries go inward as raw `Params`. |
| ME | 3 | 4 | ME/src/tags.rs:55 (`Tags(BTreeMap<TagKey, TagValue>)`) | ME/src/broker/batch_commands.rs:24 (`(&str, Params)` entries passed raw) | Typed tags, entries stay as untyped params. |
| NO | 3 | 4 | NO/src/tags.rs:37 (`Tag { key: TagKey, value }` keeps the pair together) | NO/src/tags.rs:66 (value stored as `String`) | Typed key and pair struct; untyped value and entry id. |
| QA | 3 | 4 | QA/src/tags.rs:16 (`TagKey` newtype) | QA/src/tags.rs:54 (value stored as `String`) | Typed keys and tag set; values and entry ids bare. |
| SA | 3 | 4 | SA/src/tags.rs:33 (`TagValue` with byte limit) | SA/src/broker/batch_commands.rs:21 (`(&str, Params)` entries) | Typed tags; batch entries and ids untyped. |
| WI | 3 | 4 | WI/src/tags.rs:49 (`parse_tag_object` returns typed pairs) | WI/src/broker/batch_commands.rs:98 (`required: &[&str]` list of names passed in) | Typed tags and `Entry` struct; entry params raw, id `&str`. |
| BASE | 2 | 10 | BASE/src/error.rs:7 (closed `ErrorCode` enum) | BASE/src/queue.rs:77 (`HashMap<u64, u64>`: receipt handle to arrival index, both bare `u64`) | Good enums and `Option` for dead letter; message id, handle, sequence and arrival key are all plain `u64`. |
| CU | 2 | 10 | CU/src/broker/message_commands.rs:55 (`SendArgs` struct) | CU/src/queue.rs:75 (`tags: BTreeMap<String, String>`) | Typed send arguments, but tags are validated by `check_key -> Result<()>` and stored as strings; `issued: u64, now: u64` side by side (message_commands.rs:177). |
| FA | 2 | 10 | FA/src/broker/message_commands.rs:55 (`SendParams`) | FA/src/queue.rs:76 (`tags: BTreeMap<String, String>`) | Typed send parameters, untyped tags validated and then kept as strings. |
| RI | 2 | 10 | RI/src/broker/message_commands.rs:56 (`SendParams`) | RI/src/queue.rs:85 (`tags: BTreeMap<String, String>`) | Same split as FA; `Entry` struct but id `&str`. |
| CI | 1 | 14 | CI/src/error.rs:18 (new codes added to the enum) | CI/src/broker/batch_commands.rs:71 (entry cloned into a JSON map and `QueueName` re-inserted so the single command re-parses it) | Untyped tags, JSON rebuilt and passed inward. |
| GU | 1 | 14 | GU/src/error.rs:18 (new codes added to the enum) | GU/src/broker/batch_commands.rs:82 (entry JSON cloned and `QueueName` inserted) | Same as CI; tags `BTreeMap<String, String>` (queue.rs:75). |
| HU | 1 | 14 | HU/src/tags.rs:16 (`Tags` wrapper type) | HU/src/tags.rs:35 (`set_all` takes the raw `&Map<String, Value>` and skips non-strings with `if let`) | Validated values are not kept; raw JSON goes into the domain type and relies on a precondition. |

## What separates the top from the bottom
- Parse, do not validate: the top crates turn tag keys and values into `TagKey`/`TagValue` (CA/src/tags.rs:20, QE/src/tags.rs:16, RU/src/tags.rs:18). The bottom ones call `check_key(...) -> Result<()>` and keep the `&str` (CI/src/broker/tag_commands.rs:18, CU/src/broker/tag_commands.rs:20, FA, GU, RI).
- Typed single-command arguments: CA, QE, RU (and CU, FA, RI) parse `Body`, `DelaySeconds`, `GroupId`, `DeduplicationId` into one struct that batch entries reuse. The other crates pass raw `Params` inward and re-read them per call (ME/src/broker/message_commands.rs `send_entry`).
- Raw JSON flowing inward: CI and GU clone each entry into a `serde_json::Map`, add `QueueName`, and run the single command on it (CI/src/broker/batch_commands.rs:71, GU/src/broker/batch_commands.rs:82). HU passes the raw `Tags` object into `Tags::set_all` (HU/src/tags.rs:35).
- Entry ids: only BE gives the batch `Id` its own type (BE/src/broker/batch_commands.rs:17). Everyone else carries `&str` in a tuple or a plain struct field.

## Patterns seen in most crates
- All 15 add the four batch error codes to the existing `ErrorCode` enum and never compare codes as strings.
- 12 of 15 store `max_message_bytes` as `u64` and compare it with `body.len() as u64` (for example CA/src/broker/message_commands.rs:102). Only BE and RU use `usize`, and SA and RI use checked conversions.
- Every crate builds the `Successful`/`Failed` arrays as `serde_json::Value` straight away. No crate has a typed batch outcome before serialization. This is acceptable at the response boundary but means the id/result pairing is untyped.
- No attempt changes the inherited untyped ids (`u64` message ids, receipt handles, arrival keys), which is fine for a feature task. It does keep BASE below the best attempts on the whole-crate score.
