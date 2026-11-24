# p14f / types

## Method

Read SPEC.md and SPEC-feature.md, then the whole `src/` of BASE. For each of the 12 attempts I
produced `diff -ruN BASE/src <CODE>/src` and read the full diff (every changed hunk and every new
file), looking at the same four places in each: how tags (key, value, set) are represented and
stored on `Queue`; how batch entries and their `Id` are carried from the boundary to the per-entry
handler; how `MaxMessageBytes` is parsed and stored; whether `SendMessage` parameters are parsed
into a typed value before the queue lookup. Tests were not read for this dimension. Line numbers
were confirmed with one grep over all crates. BASE was scored on its whole `src/`; attempts only on
what they added or changed.

## Scores

| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| VO | 4 | 1 | VO/src/broker/message_commands.rs:56 | VO/src/broker/batch_commands.rs:22 | TagKey, TagValue, Tags newtypes plus a typed `SendEntry` shared by single and batch send; only lapse is the entry `Id` as bare `&str` |
| KI | 4 | 1 | KI/src/broker/batch_commands.rs:17 | KI/src/tags.rs:39 | `EntryId` newtype and `Entry` struct, `TagKey`, `TagUpdate`, `Tags`; tag values stay plain `String` |
| PA | 4 | 1 | PA/src/tags.rs:16 | PA/src/broker/message_commands.rs:45 | TagKey/TagValue/Tags parsed at the boundary, `BatchEntry` struct; entry id is a bare `&str` field |
| BA | 4 | 1 | BA/src/tags.rs:20 | BA/src/broker/batch_commands.rs:25 | TagKey/TagValue/Tags parsed at the boundary; batch entries are `(&str, Params)` tuples |
| PI | 4 | 1 | PI/src/tags.rs:16 | PI/src/broker/batch_commands.rs:24 | Same shape as BA, with `TagKey::from_json`; batch entries are `(&str, Params)` tuples |
| QA | 3 | 6 | QA/src/broker/message_commands.rs:55 | QA/src/tags.rs:33 | Typed `SendArgs` and `TagKey`, but tag values are validated by a `-> Result<()>` check and stored as `String` (QA/src/queue.rs:77) |
| RE | 3 | 6 | RE/src/tags.rs:16 | RE/src/tags.rs:54 | `TagKey` and `Tags` wrapper, values bare `String`, batch entries `(&str, Params)` |
| BASE | 3 | 6 | BASE/src/attributes.rs:33 | BASE/src/queue.rs:227 | Closed ErrorCode enum, `Option<DeadLetter>` groups the paired fields; message id, handle, arrival key and sequence are all bare `u64` |
| VU | 2 | 9 | VU/src/broker/message_commands.rs:56 | VU/src/queue.rs:75 | Typed `SendArgs` and `Entry` struct, but tags are `BTreeMap<String, String>` behind a validate-only `check_key` (VU/src/broker/tag_commands.rs:18) |
| ZA | 2 | 9 | ZA/src/broker/message_commands.rs:55 | ZA/src/broker/tag_commands.rs:30 | Typed `SendArgs`, but tags parsed into `BTreeMap<String, String>` after `check_key -> ()`; `delete_in_queue` takes `issued: u64, now: u64` side by side (ZA/src/broker/message_commands.rs:175) |
| BO | 1 | 11 | BO/src/attributes.rs:131 | BO/src/broker/batch_commands.rs:70 | Tags `BTreeMap<String, String>` (BO/src/queue.rs:75); batch clones the raw JSON entry and re-injects `QueueName` to reparse it |
| LI | 1 | 11 | LI/src/attributes.rs:128 | LI/src/broker/batch_commands.rs:71 | String tag map, `check_key -> ()`, ids kept in a parallel vec and the raw entry re-cast with `expect("checked")` |
| QE | 1 | 11 | QE/src/attributes.rs:128 | QE/src/broker/batch_commands.rs:82 | String tag map, `check_key -> ()` (QE/src/broker/tag_commands.rs:18), batch rebuilds a raw JSON command per entry |

## What separates the top from the bottom

- Tag keys: the seven crates scored 3 or 4 (except BASE) parse keys into a `TagKey` newtype that can
  only be built by `parse` (KI/src/tags.rs:20); the bottom five call a `check_key(&str) -> Result<()>`
  and then store the same string (LI/src/broker/tag_commands.rs:18, ZA/src/broker/tag_commands.rs:20).
- Tag storage: a `Tags(BTreeMap<TagKey, TagValue>)` type owns the 50-tag limit in VO, PA, BA, PI
  (VO/src/tags.rs:51); the bottom crates put `pub tags: BTreeMap<String, String>` directly on
  `Queue` and enforce the limit in the handler (BO/src/queue.rs:75, QE/src/queue.rs:75).
- Batch entries: the better crates parse each entry once into an id plus a `Params` view and pass
  that inward (KI `Entry { id: EntryId, params }`, KI/src/broker/batch_commands.rs:20); BO, LI and
  QE clone the raw entry object, insert `QueueName`, and send it back through the single-command
  parser (QE/src/broker/batch_commands.rs:82), so parsing is repeated instead of carried as a value.
- Send parameters: VO, QA, VU and ZA introduce a typed send struct (`SendEntry`/`SendArgs`) parsed
  before the lookup (VO/src/broker/message_commands.rs:56); the others pass `Params` inward and read
  `Body` again inside the shared helper.
- KI is the only crate that gives the batch entry `Id` its own type (KI/src/broker/batch_commands.rs:17);
  everyone else carries it as a bare `&str`.

## Patterns seen in most crates

- All 12 extend the closed `ErrorCode` enum with the four new codes and never compare codes as
  strings.
- All 12 add `MaxMessageBytes` as an `Option<u64>`/`Option<usize>` in `AttributeUpdate`, parsed and
  range-checked at the boundary, and a plain integer in `Attributes`; none uses a sentinel.
- All 12 build `Successful`/`Failed` as `serde_json::Value` objects inline rather than typed result
  structs, following BASE's `Members` map pattern.
- None introduces newtypes for the inherited bare `u64` ids (message id, receipt handle), which is
  BASE's main lapse on this dimension.
