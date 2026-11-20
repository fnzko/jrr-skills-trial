# p2 / errors

## Method
- Read all of BASE/src (error.rs, request.rs, attributes.rs, queue.rs, naming.rs, broker.rs, broker/*.rs, main.rs) and scored it as a whole.
- For each of the 15 attempts, ran `diff -ruN BASE/src <CODE>/src` and read every added or changed line (batch, tag, MaxMessageBytes and refactored send/delete code). Inherited lines were not scored.
- Ran the same greps over every attempt's added lines: `unwrap(` / `expect(` / `unreachable` / `panic!`, ` as u..`, `unwrap_or` / `.ok()` / `let _ =`, then looked at each hit in context.
- Only `src/` was judged. Test code (where `unwrap` is normal) was left out for every crate alike.
- What I checked: Results turned into `None`/defaults, `expect`s whose invariant lives in another function or a separate pass, casts on input-derived lengths, typed errors (all use the BASE `Error { code: ErrorCode, .. }`), and branches for states that cannot happen.

## Scores
| code | score | rank | best evidence | worst evidence | reason |
|---|---|---|---|---|---|
| NO | 4 | 1 | NO/src/batch.rs:45 (a non-string `Id` is mapped to `InvalidBatchEntryId` with `map_err`, no Result dropped) | NO/src/broker/message_commands.rs:67 (`ok_or_else(missing("Body"))` repeats a `require` both callers already did; unreachable but harmless) | Checked `usize::try_from(..).expect("range is at most 1024")` (attributes.rs:131), typed tag parse (tags.rs:37), no `as` cast |
| WU | 4 | 1 | WU/src/broker/batch_commands.rs:44 (`Id` read with `get`/`as_str`, no Result thrown away) | WU/src/broker/message_commands.rs:79 (`body.len() as u64`, a lossless widening) | Every new failure is an explicit Err; `send_to` requires `Body` itself |
| VU | 4 | 1 | VU/src/broker/batch_commands.rs:44 (`Ok(None) \| Err(_)` maps both cases to the spec code explicitly) | VU/src/broker/message_commands.rs:108 (`as u64` widening) | Explicit handling throughout, invariants stay local |
| BASE | 3 | 4 | BASE/src/error.rs:7 (closed `ErrorCode` enum, typed `Error`, `expect("required")` always right after `require`) | BASE/src/broker/message_commands.rs:168 (dead-letter messages silently dropped if the target queue is missing, a state that cannot occur) | Also `max_messages as usize` (:153), and a silent `if let Some` at :196 |
| CU | 3 | 4 | CU/src/broker/tag_commands.rs:37 (tag value type and length give an explicit `InvalidParameterValue`) | CU/src/broker/batch_commands.rs:45 (`.string("Id").ok().flatten()` throws away the type error) | One dropped Result, otherwise clean |
| FI | 3 | 4 | FI/src/broker/batch_commands.rs:121 (`delete_entry` requires, then expects, in the same function) | FI/src/broker/batch_commands.rs:44 (`.identifier(..).ok().flatten()`) | Same shape as CU |
| JO | 3 | 4 | JO/src/broker/queue_commands.rs:37 (`tag_value` returns typed Err) | JO/src/broker/message_commands.rs:82 (`.ok().flatten()` on the `Id` Result) | Entries go through the single command, so its checks apply; one dropped Result |
| KE | 3 | 4 | KE/src/broker/batch_commands.rs:49 (`Id` read without a Result to drop) | KE/src/broker/batch_commands.rs:71 (`as_object().cloned().expect("checked")` relies on an earlier, separate loop) | A panic that depends on an invariant outside its own scope |
| PA | 3 | 4 | PA/src/broker/tag_commands.rs:28 (`tag_value` returns typed Err) | PA/src/broker/batch_commands.rs:47 (`.ok().flatten()`) | One dropped Result |
| PI | 3 | 4 | PI/src/broker/tag_commands.rs:40 (separate errors for a non-string tag value and an overlong one) | PI/src/broker/batch_commands.rs:40 (`.ok().flatten()`) | One dropped Result |
| PO | 3 | 4 | PO/src/broker/batch_commands.rs:86 (entry failures carried as typed code and message) | PO/src/broker/batch_commands.rs:41 (`.ok().flatten()`, which the comment says is deliberate) | One dropped Result |
| RO | 3 | 4 | RO/src/broker/batch_commands.rs:85 (failed entries keep their typed error) | RO/src/broker/batch_commands.rs:41 (`.ok().flatten()`) | One dropped Result |
| VI | 3 | 4 | VI/src/tags.rs:57 (`set_all` makes the limit check part of the tag type) | VI/src/broker/batch_commands.rs:40 (`.ok().flatten()`; also `try_from(..).map_or(true, ..)` at message_commands.rs:80, a branch for a failure that cannot happen) | Checked conversion, but one dropped Result |
| XO | 3 | 4 | XO/src/tags.rs:70 (typed `Tag`/`TagKey`, limit inside `set_all`) | XO/src/broker/message_commands.rs:64 (`send_to` does `expect("required")` on `Body`, which its callers must `require` first) | A panic precondition that crosses a function boundary |
| XA | 2 | 15 | XA/src/broker/tag_commands.rs:37 (typed tag value check) | XA/src/broker/message_commands.rs:65 (`SendArgs::parse` expects a `Body` its caller must require) plus batch_commands.rs:42 `.ok().flatten()` | Two kinds of lapse |
| ZU | 2 | 15 | ZU/src/broker/queue_commands.rs:180 (tag value check returns typed Err) | ZU/src/broker/message_commands.rs:306 (`expect("checked by batch_ids")` re-derives each entry after validation; results are matched to ids by `zip` at :94, which silently truncates on a length mismatch) | Also `.ok().flatten()` (:70), and `entries()` drops null tag values without an error (request.rs:147) |

## What separates the top from the bottom
- The top three keep every new failure as an explicit `Err`. On the entry `Id` they avoid the Result entirely (WU/src/broker/batch_commands.rs:44), map it with `map_err` (NO/src/batch.rs:45) or match on it (VU/src/broker/batch_commands.rs:44). Most others write `.ok().flatten()` (CU/src/broker/batch_commands.rs:45 and seven more).
- At the bottom, panics rest on invariants held somewhere else. ZU and KE validate entries in one pass and then `expect` the object again in a second (ZU/src/broker/message_commands.rs:306, KE/src/broker/batch_commands.rs:71). XA and XO move the `require("Body")` out of the function that `expect`s it (XA/src/broker/message_commands.rs:65, XO/src/broker/message_commands.rs:64).
- ZU also pairs ids with results through a separate `zip` (ZU/src/broker/message_commands.rs:94). Everyone else carries the id together with its entry, so no mismatch is possible.
- Only NO and VI handled the `MaxMessageBytes` comparison without an `as` cast (NO keeps the field as `usize`; VI uses `try_from`). The other 13 write `body.len() as u64`, which is lossless but is the cast the brief asks to avoid.

## Patterns seen in most crates
- All of them reuse BASE's typed `Error { code: ErrorCode, message }` and add the four batch codes to the enum. None uses `String` errors, and every failed entry serializes the typed code.
- All of them check entry and tag values at step 3 and return `InvalidParameterValue` or the batch codes before mutating anything. Tag limits are checked before any insert, so a failed `TagQueue` changes nothing.
- None of them has a bare `unwrap` in `src`. Every `expect` is `"required"` right after `require`, except the cross-scope ones named above.
- BASE's lapses are inherited by all of them and not counted against attempts: the silent dead-letter drop, the silent `if let Some(message)` in `ChangeMessageVisibility`, and `max_messages as usize`.
