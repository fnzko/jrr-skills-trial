# p7f / errors

## Method

Read only `v2/blind/p7f/` and the two spec files. Read all of `BASE/src` for its error baseline
(typed `Error { code: ErrorCode, message }`, checked integer parsing in `request.rs`, `expect("required")`
right after `require`). For each of the 21 attempts, ran `diff -ruN BASE/src <CODE>/src` and read the
whole diff (new batch and tag modules, the `SendMessage`/`DeleteMessage` split, `MaxMessageBytes`,
request helpers). Tests were not scored for this dimension. Then ran the same greps over every crate's
`src`: `as u64`, `.ok()`, `map_or(true`, `unwrap_or(`, `expect("...")` and how each send/delete path
gets `Body`/`ReceiptHandle` (is the `expect` backed by a `require` in the same function, a named
contract, or a caller in another module). Every crate got the same reads and greps. Judged each attempt
on the code that differs from `BASE`. `BASE` was scored on its own source.

Things I looked for in the new code: per-entry and per-command atomicity (nothing changes on failure),
`Result` turned into `Option` with `.ok()`, `as` casts on input-derived lengths, `expect` whose
invariant is held in another function, branches for impossible states, silent skips.

## Scores

| code | score | rank | best evidence | worst evidence | one-line reason |
|---|---|---|---|---|---|
| WA | 4 | 1 | WA/src/attributes.rs:131 | WA/src/broker/batch.rs:73 | checked `usize::try_from` with a named invariant, `missing("Body")` instead of `expect`, `map_err` keeps the Id result in `Result` |
| LA | 4 | 2 | LA/src/broker/batch_commands.rs:52 | LA/src/broker/message_commands.rs:72 | reads `Id` raw so no error is dropped, changed even the BASE body check to `try_from`; only lapse is a `map_or(true)` branch that cannot run |
| BA | 4 | 2 | BA/src/broker/message_commands.rs:64 | BA/src/broker/message_commands.rs:79 | callee requires its own `Body`, so every `expect` sits next to its check; explicit `Ok(None) \| Err(_)` mapping; one `as u64` |
| VI | 4 | 2 | VI/src/broker/batch_commands.rs:34 | VI/src/broker/message_commands.rs:79 | raw `Id` read, `require` inside `send_to`/`delete_from`; one `as u64` |
| SO | 4 | 2 | SO/src/broker/message_commands.rs:68 | SO/src/broker/message_commands.rs:99 | `expect` names its contract ("caller checked `Body` is present"), `let Ok(Some(id)) else`; one `as u64` |
| BASE | 4 | 2 | BASE/src/request.rs:100 | BASE/src/broker/message_commands.rs:153 | checked integer conversion at the boundary, typed codes; `max_messages as usize` on a range-checked value |
| KI | 3 | 7 | KI/src/broker/tag_commands.rs:58 | KI/src/broker/batch_commands.rs:47 | reruns the single command so errors match exactly; `.ok().flatten()` on `Id`, dead `map_or(true)` |
| RO | 3 | 7 | RO/src/tags.rs:65 | RO/src/broker/batch_commands.rs:40 | typed `TagKey`/`TagValue` parsing with `Result`, atomic `set_all`; `.ok().flatten()`, dead `map_or(true)` |
| WI | 3 | 7 | WI/src/broker/batch_commands.rs:30 | WI/src/broker/message_commands.rs:65 | `map_err` keeps the Id in `Result`; `send_to` `expect`s a `Body` its caller in another module required, dead `map_or(true)` |
| NE | 3 | 10 | NE/src/broker/message_commands.rs:66 | NE/src/broker/batch_commands.rs:52 | no `expect` on entry fields (`ok_or_else(missing)`); `.ok().flatten()` and `as u64` |
| ME | 3 | 10 | ME/src/broker/message_commands.rs:66 | ME/src/broker/batch_commands.rs:45 | `require` inside `SendArgs::parse`; `.ok().flatten()`, `as u64` |
| MI | 3 | 10 | MI/src/broker/message_commands.rs:64 | MI/src/broker/batch_commands.rs:51 | self-contained `send_one`; `.ok().flatten()`, `as u64` |
| NO | 3 | 10 | NO/src/broker/message_commands.rs:66 | NO/src/broker/batch_commands.rs:25 | `require` inside parse; `parse_entry` returns `Option`, throwing the error away with `Err(_) => None`; `as u64` |
| ZI | 3 | 10 | ZI/src/broker/tag_commands.rs:57 | ZI/src/broker/batch_commands.rs:40 | exhaustive match on tag value kind, reruns the single command; `.ok().flatten()`, `as u64` |
| LU | 3 | 10 | LU/src/broker/batch_commands.rs:42 | LU/src/broker/message_commands.rs:65 | raw `Id` read; `send_entry`/`delete_entry` `expect` fields their caller required, `as u64` |
| PO | 3 | 10 | PO/src/broker/batch_commands.rs:46 | PO/src/broker/message_commands.rs:64 | explicit match on the Id result, typed tags; callee `expect`s across modules, `as u64` |
| NA | 3 | 17 | NA/src/broker/tag_commands.rs:55 | NA/src/broker/batch_commands.rs:47 | contract documented on `parse`, but `.ok().flatten()`, `as u64`, cross-module `expect` |
| FU | 3 | 18 | FU/src/broker/tag_commands.rs:51 | FU/src/broker/message_commands.rs:67 | atomic tag limit; unnamed cross-module `expect`, `.ok().flatten()`, `as u64` |
| RE | 3 | 18 | RE/src/broker/tag_commands.rs:59 | RE/src/broker/batch_commands.rs:43 | separate not-a-string vs too-long errors; `.ok().flatten()`, `as u64`, unnamed cross-module `expect` |
| BU | 3 | 18 | BU/src/broker/message_commands.rs:67 | BU/src/broker/message_commands.rs:102 | named `expect`; length conversion fails into `unwrap_or(u32::MAX)`, `.ok().flatten()` on `Id` |
| CA | 2 | 21 | CA/src/tags.rs:43 | CA/src/tags.rs:47 | `set_all` takes raw JSON and silently skips non-string values (validation lives elsewhere); plus `.ok()`, `as u64`, cross-module `expect` |
| SU | 2 | 21 | SU/src/broker/batch_commands.rs:49 | SU/src/broker/batch_commands.rs:71 | throws away the validated object, then gets it back with `as_object().cloned().expect("checked")`; `as u64` |

## What separates the top from the bottom

- Where the invariant lives. At the top, every `expect` sits next to the check that makes it hold:
  BA/src/broker/message_commands.rs:64, VI, MI, RO put `require(&["Body"])` inside the callee, and
  NE/src/broker/message_commands.rs:66 and WA/src/broker/message_commands.rs:67 use `ok_or_else(missing)`
  with no `expect` at all. Lower down, a callee in `message_commands.rs` `expect`s a field that the
  batch closure in another module required (CA:64, LU:65, PO:64, WI:65, FU:67, RE:66), with the
  generic message "required".
- Re-checking proven state. SU/src/broker/batch_commands.rs:71 drops the object it already checked
  and gets it back with `expect("checked")`. CA/src/tags.rs:47 has `if let Some(text)` that silently
  skips a value whose validity is guaranteed only by a caller in another file. These are the only
  structural lapses in the set.
- Numeric conversion of the body length. WA keeps `max_message_bytes` as `usize` and converts the
  attribute once with a named invariant (WA/src/attributes.rs:131). LA, KI, RO and WI use `try_from`
  but add a `map_or(true, ...)` branch that cannot run. BU converts through `u32` and turns a failure
  into `u32::MAX` (BU/src/broker/message_commands.rs:102). The other 15 cast `body.len() as u64`.
- Dropping the `Id` result. 11 attempts turn `params.identifier/string("Id")` into an `Option` with
  `.ok().flatten()` (e.g. KI/src/broker/batch_commands.rs:47). NO does it with `Err(_) => None`.
  LA, LU, SU and VI read `Id` from the raw map, so there is no error to drop. WA and WI keep it a
  `Result` with `map_err`, and BA, PO and SO match the result explicitly.

## Patterns seen in most crates

- All 21 keep `BASE`'s typed `ErrorCode`/`Error` and add the four batch codes as variants. None
  introduces `String` errors, `unwrap()`, or panicking indexing in production code. The two
  `parsed[..i]` slices (ME:52, NA:55) are bounded by `enumerate`.
- All 21 are atomic where the spec requires it: TagQueue validates every tag and checks the 50-tag
  limit before inserting anything, batch rules 1 to 4 run before any entry, and a failing entry
  returns `Err` before it mutates anything. No attempt drops a per-entry `Result`. Each one goes into
  `Failed`.
- All 21 map every malformed-entry case (`null`, non-string, bad characters) to
  `InvalidBatchEntryId` and check the queue lookup before the numbered rules.
- The remaining lapses are small and repeated across most crates: lossless `usize as u64` casts and
  `Result` to `Option` conversions on the entry `Id`.
