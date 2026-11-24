# p14f / errors

## Method
- BASE: read every file in `src/` in full.
- Each attempt: read the full `diff -ruN BASE/src <CODE>/src`, so every added or changed line got the same read. Tests were not scored.
- Then ran the same grep over every crate's `src/` for `as u64|as usize|try_from|expect(|.ok()|Err(_)|if let Some(message)`. This located each `expect`, cast and swallowed error and gave the line numbers.
- Attempts are scored only on added or changed code. Inherited BASE lines count neither for nor against them.

## Scores
| code | score | rank | best evidence | worst evidence | reason |
|---|---|---|---|---|---|
| BA | 4 | 1 | BA/src/broker/message_commands.rs:64 | BA/src/broker/batch_commands.rs:45 | New paths use `required_string` (MissingParameter, no `expect`) and `try_from` for the size check; the only lapse is a deliberate collapse of an `Id` type error into `InvalidBatchEntryId`, which the spec asks for |
| BASE | 3 | 2 | BASE/src/request.rs:100 | BASE/src/broker/message_commands.rs:196 | Typed `ErrorCode` and checked integer parsing; every `expect("required")` follows a `require` in the same function; one `if let` covers a message that cannot be missing |
| BO | 3 | 2 | BO/src/broker/message_commands.rs:72 | BO/src/broker/batch_commands.rs:31 | Uses `u64::try_from` for both body checks; keeps the require-then-`expect("required")` pattern in every new handler |
| KI | 3 | 2 | KI/src/request.rs:85 | KI/src/attributes.rs:134 | Adds `required_string`/`get_required` with typed errors and a typed `EntryId`; adds an error branch for `usize::try_from` on a value already bounded to 1024, which cannot fail |
| PA | 3 | 2 | PA/src/broker/message_commands.rs:135 | PA/src/broker/message_commands.rs:121 | `try_from` size check and `required_*` helpers; `send_to_queue` still calls `expect("required")` and relies on the caller (another function) having run `require`. The contract is written only in a doc comment |
| PI | 3 | 2 | PI/src/tags.rs:76 | PI/src/broker/message_commands.rs:79 | Validates tags fully before mutating, and each `expect` sits right after its `require`; the size check uses `body.len() as u64` |
| QA | 3 | 2 | QA/src/broker/message_commands.rs:64 | QA/src/broker/message_commands.rs:99 | Replaced the send-path `expect`s with `require_string` through a typed `SendArgs`; `as u64` cast on the body length |
| QE | 3 | 2 | QE/src/broker/batch_commands.rs:89 | QE/src/broker/message_commands.rs:72 | Per-entry errors are kept and reported with their codes; `as u64` cast, and `.ok().flatten()` folds the `Id` error away (allowed by the spec) |
| RE | 3 | 2 | RE/src/request.rs:139 | RE/src/broker/message_commands.rs:80 | `required_*` helpers with typed errors, and local require-then-expect; `as u64` cast |
| VO | 3 | 2 | VO/src/broker/batch_commands.rs:71 | VO/src/broker/message_commands.rs:102 | Batch paths use `required_string`; typed `TagKey`/`TagValue`; `as u64` cast |
| LI | 2 | 11 | LI/src/broker/batch_commands.rs:61 | LI/src/broker/batch_commands.rs:71 | Throws away the checked object and re-derives it with `.expect("checked")`; `as u64` cast; `expect("required")` throughout the new tag handlers |
| VU | 2 | 11 | VU/src/broker/batch_commands.rs:58 | VU/src/broker/message_commands.rs:67 | `SendArgs::parse` calls `expect("required")`, which depends on every caller having run `require` first; `as u64` cast; plus more `expect`s in batch and tags |
| ZA | 1 | 13 | ZA/src/broker/tag_commands.rs:71 | ZA/src/broker/tag_commands.rs:31 | `parse_tags`/`parse_keys` call `expect("required")` and depend, with nothing written down, on `require` in their caller; about ten new `expect`s; `as u64`; `.ok().flatten()` |

## What separates the top from the bottom
- How "present" is guaranteed. BA, KI, QA, RE and VO return MissingParameter from `required_string`/`required_array`, so a later refactor cannot turn a missing member into a panic (BA/src/broker/message_commands.rs:64). The bottom crates rely on `expect("required")`, and some of those sit in helper functions far from the `require` they depend on (ZA/src/broker/tag_commands.rs:31, VU/src/broker/message_commands.rs:67, PA/src/broker/message_commands.rs:121).
- Checked versus `as` conversion on input length. BA, BO and PA use `u64::try_from(body.len())` (BO/src/broker/message_commands.rs:72). PI, QA, QE, RE, VO, VU, LI and ZA cast with `as u64`. The cast cannot fail on 64-bit targets, but the rubric asks for checked conversion.
- Re-deriving checked data with `expect` instead of keeping the typed value. LI drops the parsed object and gets it back with `.expect("checked")` (LI/src/broker/batch_commands.rs:71). Every other crate carries `(id, params)` forward.
- Branches for states that cannot occur. KI adds a `usize::try_from` failure path for a value already bounded to 1024 (KI/src/attributes.rs:134). BASE has the inherited `if let Some(message)` after an in-flight lookup (BASE/src/broker/message_commands.rs:196).

## Patterns seen in most crates
- All of them extend the typed `ErrorCode` enum with the four batch codes. None uses `String` errors, drops a `Result`, or calls `unwrap` in non-test code.
- All of them validate tags completely and check the 50-tag limit before changing anything, so a failed command changes no state.
- Most of them turn the `Id` type error into `InvalidBatchEntryId` via `.ok()` or `Err(_)`. The spec requires that code, so it is not counted against them.
- Untouched inherited lapses (unchecked `now + seconds`, `max_messages as usize`) remain in every crate. They were not scored on the attempts.
