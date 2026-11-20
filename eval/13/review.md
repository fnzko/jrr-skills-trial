# Review of run 13

## Summary
The attempt split `state.rs` (4877 -> 3292 lines) by moving seven cohesive blocks into new or existing
`state/` submodules (batch limits, queue attribute specs, tags, receipt handles, queue addressing,
message move task, message attribute digests, redrive member names), each re-exported through
`pub use x::*` in the pattern the crate already used for `permissions` and `redrive`. A line-multiset
comparison of every move commit shows the moved code is verbatim apart from new module docs and a
handful of private items widened to `pub(super)`. The one non-move commit drops the 23 inert
`#[allow(unused_imports)]` attributes from the op files. Everything else was left alone: no newtypes,
the string attribute map, giant handlers, duplicated `lift`/`store_unavailable`, `expect`s and comment
volume are untouched. Two side effects hurt: a stray `state/message_move_task.rs-E` (a sed backup) is
committed, and the "Store bounds" banner was cut off from the constants it heads.

## Commit classification
| hash | subject | kind | pure move? |
|---|---|---|---|
| 4ab882a1b | move message attribute digest and size helpers next to their rules | move | yes |
| 8dd03d03b | move the message move task record into its own module | move | no (3 fns -> pub(super), banner dropped, stray `.rs-E` file added) |
| 0100201f1 | move queue addressing into state/queue_address.rs | move | yes (1 fn -> pub(super)) |
| 8554de138 | move receipt handle helpers into state/receipt_handle.rs | move | yes (const + fn -> pub(super)) |
| a63fc8ef3 | move tag quotas and refusals into state/tags.rs | move | yes |
| a61889fa2 | keep the redrive attribute names with the redrive rules | move | yes (2 consts -> pub(super)) |
| e4cf18e0b | move queue attribute specs and validation into state/queue_attributes.rs | move | yes (but carries the "Store bounds" banner along) |
| a8a2f5348 | drop allow(unused_imports) from operation modules | other (lint) | n/a |
| 2961d9c2a | move batch limits and refusal wording into state/batch.rs | move | yes |

## Scores
| dimension | score | evidence |
|---|---|---|
| 1. Structure | +1 | state.rs 4877 -> 3292 lines; cohesive private modules `state/batch.rs:1-91`, `state/tags.rs`, `state/receipt_handle.rs`, `state/queue_address.rs` (commits 2961d9c2a, a63fc8ef3, 8554de138, 0100201f1); widening limited to `pub(super)` (`state/message_move_task.rs:159,164,195`). Glob re-exports (`state.rs:210-219`) keep a flat namespace; glob use rate 4.03 -> 4.63/kLOC. |
| 2. Duplication | 0 | Only relocation; no helper merged. `fn lift`/`store_unavailable` per op unchanged. |
| 3. Error handling | 0 | No error-path code changed. |
| 4. Domain types | 0 | `queue_key` still `(String, String)` (`state/queue_address.rs`), `is_of(&self, key: &(String, String))` at `state/message_move_task.rs:159`. |
| 5. Ownership | 0 | No clone/to_string changes (metrics within rounding). |
| 6. Comments | -1 | e4cf18e0b moved the "Store bounds" banner to `state/queue_attributes.rs:274-283`, where it heads attribute refusals, while `MAX_QUEUES` stays at `state.rs:389` and `state.rs:152-153` still says "see the Store bounds section"; a63fc8ef3 left `state.rs:549` "tag quotas above" though they now live in `state/tags.rs`. Each new module also gets its summary twice (`//!` in the file and `///` on the `mod` line, `state.rs:180-199`). Comment share unchanged at 43.4%. |
| 7. Control flow | 0 | No function rewritten. |
| 8. Lint suppressions | +1 | a8a2f5348 removes `#[allow(unused_imports)]`, e.g. `ops/send_message.rs:163`; clippy `allow_attributes` 142 -> 96, `allow_attributes_without_reason` 196 -> 150. |
| 9. Tests | 0 | No test touched; 280 passed before and after. |
| 10. Commit hygiene | +1 | One move per commit with bodies that say why and name visibility changes (e.g. 8554de138, a61889fa2). Marred by 8dd03d03b committing `state/message_move_task.rs-E`, a 253-line pre-edit sed backup still in HEAD. |
| 11. Behavior risk | 0 | Line-multiset diff of every move shows only docs, `mod`/`pub use`/`use super::*` lines and `pub(super)` visibility; the `.rs-E` file is not a module, so not compiled. |

## Audit findings
| id | status | evidence |
|---|---|---|
| S-01 | untouched | |
| S-05 | untouched | spec table moved to `state/queue_attributes.rs`, string map and `integer()` unchanged |
| S-07 | untouched | |
| S-08 | untouched | |
| S-09 | untouched | |
| S-12/S-13 | untouched | |
| S-14/S-15 | untouched | |
| S-16/S-17 | partial | a8a2f5348 drops all 23 `#[allow(unused_imports)]` (S-17); per-op `lift`/`store_unavailable` (S-16) remain |
| S-18 | untouched | |
| S-19 | untouched | |
| S-20 | untouched | |
| S-21 | untouched | |

## Best hunks
a8a2f5348, `ops/send_message.rs:163` (same hunk in 23 op files):
```
-#[allow(unused_imports)]
 use super::types::*;
 use crate::scaffolded::Scaffolded;
```
Removes a suppression that could never fire and only masked future dead imports.

2961d9c2a, `state/batch.rs:1-17`:
```
//! The limits a batch request is held to (entry count, entry id, payload size) and the
//! wording of the refusals for breaking them.

use super::*;

pub const MAX_BATCH_ENTRIES: usize = 10;
pub const MAX_BATCH_ENTRY_ID_LEN: usize = 80;
```
(doc comments elided) One rule set shared by three batch ops now sits in a small module that reads on its own.

## Worst hunks
e4cf18e0b, `state/queue_attributes.rs:274-286`:
```
// ---------------------------------------------------------------------------
// Store bounds
// ---------------------------------------------------------------------------
//
// The store is in-memory and the emulator authenticates nobody, so everything a
// caller can make it hold is bounded. ...

/// The modelled refusal for an attribute value the call does not accept: ...
pub fn invalid_attribute_value(attribute: &str) -> InvalidAttributeValue {
```
The banner now heads attribute refusals in another file, while the store bounds it introduces (`MAX_QUEUES`, `state.rs:389`) stayed behind headerless.

8dd03d03b, `state/message_move_task.rs-E:159` (new file):
```
    fn is_of(&self, key: &(String, String)) -> bool {
```
A 253-line uncompiled duplicate of the module (the pre-`pub(super)` copy left by `sed -i -E`) is committed and survives to HEAD.

## Silent edits check
Compared every move commit mechanically (sorted removed vs added lines, leading whitespace stripped):
tag quotas and refusal functions (a63fc8ef3), `encoded_opaque_token` and handle
helpers (8554de138), `queue_arn_fields`, `queue_key` and URL parsing (0100201f1), `MessageMoveTask`
methods `deliverable_by`/`in_progress`/`is_of` (8dd03d03b), the `SETTABLE_ATTRIBUTES` table and
`check_attribute_value` (e4cf18e0b), `is_valid_batch_entry_id`/`empty_batch_message` (2961d9c2a),
the digest helpers (4ab882a1b) and the redrive member constants (a61889fa2). The only differences are
added `//!`/`///` module docs, `mod`/`pub use`/`use super::*` lines, one dropped empty banner in
8dd03d03b, and `pub(super)` on the items named in the commit bodies. No body edits found.
