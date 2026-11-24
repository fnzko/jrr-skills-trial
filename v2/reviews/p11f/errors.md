# p11f / errors

## Method

- Read the feature spec and all of `BASE/src` line by line to set the baseline.
- For each of the 15 attempts, produced `diff -ruN BASE/src <CODE>/src` and read the whole diff (new files included). Only added or changed lines were scored; inherited code an attempt left untouched did not count.
- Ran the same grep over every crate's `src` for: `as u64`, `try_from`, `.ok()`, `.ok().flatten()`, `Err(_) =>`, `map_err(|_|`, `expect(`, `if let Some(` on values that were validated earlier, and silent skips. Every crate got the same searches.
- Test files were not scored (`unwrap` in tests is fine). Cargo was not run.
- The things checked in new code: casts and conversions on input-derived numbers; `Result`s discarded or folded into `Option`; `expect`s and whether they name the invariant they rely on; validation separated from use, so that a later step has to assume or silently skip; branches for states that cannot occur; typed error codes.

## Scores

| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| RU | 4 | 1 | RU/src/attributes.rs:131 | RU/src/broker/queue_commands.rs:186 | `usize` field, `try_from(...).expect("range is at most 1024")` names the invariant; `Body` missing is a real `MissingParameter` (message_commands.rs:67), not an `expect`; only lapse: limit check and insert are split across two calls |
| SA | 4 | 1 | SA/src/broker/message_commands.rs:80 | SA/src/broker/batch_commands.rs:40 | checked `u64::try_from` on the body length that fails closed; typed TagKey/TagValue; `.ok().flatten()` on `Id` is commented and maps to the right code |
| BASE | 3 | 3 | BASE/src/queue.rs:234 | BASE/src/broker/message_commands.rs:196 | typed `ErrorCode`, checked integer parsing, named `expect`; `if let Some` for a key just found, unchecked `self.now += seconds` (broker.rs:87) and `max_messages as usize` (message_commands.rs:153) |
| BE | 3 | 3 | BE/src/tags.rs:71 | BE/src/attributes.rs:134 | `usize` field avoids the cast and the tag limit is all-or-nothing; adds an error branch for `usize::try_from` on a value already capped at 1024, which cannot fail |
| CA | 3 | 3 | CA/src/tags.rs:36 | CA/src/broker/message_commands.rs:102 | typed `TagKey`/`TagValue`, `required_string` returns `MissingParameter` rather than panicking; `body.len() as u64` |
| CI | 3 | 3 | CI/src/broker/batch_commands.rs:47 | CI/src/broker/batch_commands.rs:71 | `Id` rule maps cleanly to one error; re-derives the entry object after validation with `.expect("checked")` instead of keeping it; `as u64` (message_commands.rs:72) |
| FA | 3 | 3 | FA/src/broker/batch_commands.rs:21 | FA/src/broker/message_commands.rs:102 | `parse_entry` documents that it returns `None` for an invalid entry and the caller maps that to the right code (`Err(_) => None` at :25); `as u64` |
| GU | 3 | 3 | GU/src/broker/batch_commands.rs:20 | GU/src/broker/batch_commands.rs:40 | batch rules checked before any entry runs; `.ok().flatten()` folds a typed error away; `as u64` (message_commands.rs:72) |
| ME | 3 | 3 | ME/src/broker/message_commands.rs:65 | ME/src/broker/message_commands.rs:79 | `expect("caller required Body")` states the contract; typed tags with `parse_given`; `as u64` |
| NO | 3 | 3 | NO/src/tags.rs:37 | NO/src/broker/message_commands.rs:78 | `Tag` type carries a validated key and value into `set_all`; `as u64` |
| QA | 3 | 3 | QA/src/request.rs:139 | QA/src/broker/batch_commands.rs:50 | `required_string`/`required_array` return `MissingParameter`, and the entry helpers call `require` themselves before `expect`; `.ok().flatten()` and `as u64` (message_commands.rs:80) |
| QE | 3 | 3 | QE/src/tags.rs:31 | QE/src/broker/message_commands.rs:99 | `from_json` constructors turn wrong JSON types into errors; `expect("caller checked Body is present")` (message_commands.rs:68); `as u64` |
| RI | 3 | 3 | RI/src/broker/message_commands.rs:67 | RI/src/broker/message_commands.rs:102 | named `expect`; the length check uses `u32::try_from(..).unwrap_or(u32::MAX)`, which turns a failed conversion into a default value (it only stays correct by accident) |
| WI | 3 | 3 | WI/src/tags.rs:33 | WI/src/broker/message_commands.rs:79 | typed tags with fallible parsers, `Ok(Some(id))` match at batch_commands.rs:48; `as u64` |
| CU | 2 | 15 | CU/src/broker/message_commands.rs:188 | CU/src/broker/tag_commands.rs:31 | `parse_tags`/`parse_keys` `expect("required")` on a `require` done in a different function; `.ok().flatten()` (batch_commands.rs:45); rewrote `change_message_visibility` but kept the impossible `if let Some` (message_commands.rs:279); `as u64` |
| HU | 2 | 15 | HU/src/broker/batch_commands.rs:36 | HU/src/tags.rs:47 | `Tags::set_all` takes raw JSON and silently skips non-string values, relying on a separate `check_value` call (tags.rs:34); `send_to_queue` `expect("required")` (message_commands.rs:64) depends on every caller calling `require` first; `as u64` |

## What separates the top from the bottom

- **Checked numeric conversion.** RU stores `max_message_bytes` as `usize` and converts once with `try_from(...).expect("range is at most 1024")` (RU/src/attributes.rs:131). SA uses `u64::try_from(body.len()).map_or(true, ...)`, which fails closed (SA/src/broker/message_commands.rs:80). Eleven attempts compare with `body.len() as u64`, for example CA/src/broker/message_commands.rs:102. RI converts but swallows the failure with `unwrap_or(u32::MAX)` (RI/src/broker/message_commands.rs:102).
- **Validated values carried in types instead of re-checked later.** NO (`Tag`, NO/src/tags.rs:37), ME, QE, SA, WI and CA pass `TagKey`/`TagValue` into the store, so the store cannot see bad data. HU passes the raw `Map` into `set_all` and drops non-strings with `if let Some(text)` (HU/src/tags.rs:47). That is a silent default if a caller ever forgets `check_value`.
- **Where `expect` relies on a precondition, the precondition is close and named.** ME (`"caller required Body"`, ME/src/broker/message_commands.rs:65), QE and RI name the contract. RU returns `MissingParameter` instead (RU/src/broker/message_commands.rs:67). CU puts `expect("required")` in helpers whose `require` call lives in another function (CU/src/broker/tag_commands.rs:31). CI re-derives an object it already validated (CI/src/broker/batch_commands.rs:71).
- **Impossible branches.** BE adds an error path for `usize::try_from` on a value already capped at 1024 (BE/src/attributes.rs:134). CU carries BASE's impossible `if let Some(message)` into a function it rewrote (CU/src/broker/message_commands.rs:279).

## Patterns seen in most crates

- All 15 attempts added the four batch codes to the `ErrorCode` enum. No attempt introduced `String` errors, `unwrap`, or panics reachable from request input in the new code.
- Nearly all of them validate the whole batch (rules 1 to 4) before running any entry, and return each entry failure as data in `Failed` instead of aborting.
- The common lapse is folding the `Id` type error away with `.ok().flatten()`, `Err(_) => None` or a `_ =>` arm (CU, GU, HU, QA, QE, RI, SA, FA, ME, NO, WI). The result is spec-correct, because any bad `Id` becomes `InvalidBatchEntryId`, so this was weighted lightly.
- The common numeric lapse is `body.len() as u64`. It is lossless on 64-bit targets but is not a checked conversion. Only RU, SA and BE avoid it.
- All of them keep BASE's `require` then `.expect("required")` idiom. Most new helpers keep the `require` in the same function as the `expect`.
