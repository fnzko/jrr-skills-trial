# Review of run 4

## Summary
The attempt is a pure file split of `src/state.rs` (4877 lines at baseline, 655 at HEAD) into eleven new
private submodules (`address`, `attributes`, `clock`, `digest`, `limits`, `message`, `message_move`,
`queue`, `queue_rules`, `receipt_handle`, `refusals`), and it moves more items into the existing
`message_attributes` and `redrive` modules. Each new module follows the baseline's own `mod x; pub use x::*;`
convention, gets a `//!` header, and starts with `use super::*;`. The only non-move edits are six items raised to
`pub(super)`, the old `// ----` banner comments turned into module docs, a "Where things live" map in the
`state` docs, and two `///` docs on private `mod` lines dropped because they repeat the target file's `//!`.
It changes no function body, type, test, lint suppression or comment volume. None of the audit findings
(string keys, string attribute maps, sweeps, long-poll loop, scans, `expect` after lookup, giant
handlers, comment volume) is touched.

## Commit classification
| hash | subject | kind | pure move? |
|---|---|---|---|
| 38a2735c9 | move message attribute digests and limits into message_attributes | move | yes |
| 697c442a2 | extract digests and identifier generation into state::digest | move | yes |
| 11250aab2 | extract time arithmetic and epoch rendering into state::clock | move | yes (+1 pub(super)) |
| 3e3c5ab7c | extract receipt handle format and refusals into state::receipt_handle | move | yes (+2 pub(super)) |
| f8371192b | extract refusal wording into state::refusals | move | yes |
| 1a4e9f707 | extract queue addressing into state::address | move | yes (+1 pub(super)) |
| edca95bab | extract queue attribute specs and validation into state::attributes | move | yes (+2 pub(super)) |
| 7292c6274 | extract queue name, tag and re-create rules into state::queue_rules | move | yes |
| d40b5167e | extract store ceilings into state::limits | move | yes (banner -> //!) |
| a1d12fc56 | extract the stored message records into state::message | move | yes |
| 29b2ac3b7 | move the Queue record and its impl into state::queue | move | yes |
| 850637adc | move message movement tasks into state::message_move | move | yes |
| d36162e26 | move the attribute refusal constructors next to the attribute checks | move | yes |
| fc36aa875 | move tag quotas next to the tag rules in state::queue_rules | move | yes |
| d3971d58f | move the store-wide redrive checks into state::redrive | move | yes |
| c91018b71 | order state module declarations and document the layout | other (docs/order) | yes |

## Scores
| dimension | score | evidence |
|---|---|---|
| Structure | +2 | 4877-line `state.rs` -> 655 lines plus cohesive files (c91018b71 `state.rs:183-231`, 29b2ac3b7 `state/queue.rs`, 850637adc `state/message_move.rs:8`); visibility kept tight: `pub(crate)` count 1 -> 1, only 6 new `pub(super)` (e.g. `state/address.rs:171`, `state/clock.rs:9`). Minus: 11 more `use super::*` globs (glob use 4.03 -> 5.13/kLOC), `queue.rs` still 1165 lines |
| Duplication | 0 | nothing removed, only relocated |
| Error handling | 0 | expect rate unchanged (0.71/kLOC); `Queue::integer` default still at `state/queue.rs:281` |
| Domain types | 0 | `queues: BTreeMap<(String, String), Queue>` still at `state.rs:415`; `queue_key` still allocates at `state/address.rs:14-16` |
| Ownership | 0 | clone/to_string rates unchanged |
| Comments | 0 | comment share 43.4% -> 43.3%; banners 0.61 -> 0.20/kLOC as banners became `//!` (d40b5167e `state/limits.rs:3-6`), net negligible |
| Control flow | 0 | no function body changed (verified per commit, see below) |
| Lint suppressions | 0 | `clippy.txt` identical (360 total, same breakdown) |
| Tests | 0 | no test touched; 280/280 pass as at baseline |
| Commit hygiene | +2 | 16 one-move commits, each body says why the items belong together (e.g. 850637adc, edca95bab), and names each visibility change it makes |
| Behavior risk | 0 | per-commit line multisets match apart from `mod`/`use` plumbing, `pub(super)` and comments |

## Audit findings
| id | status | evidence |
|---|---|---|
| S-01 | untouched | |
| S-05 | untouched | |
| S-07 | untouched | |
| S-08 | untouched | |
| S-09 | untouched | |
| S-12/S-13 | untouched | |
| S-14/S-15 | untouched | |
| S-16/S-17 | untouched | |
| S-18 | untouched | |
| S-19 | untouched | |
| S-20 | untouched | |
| S-21 | untouched | |

## Best hunks
c91018b71, `state.rs:156-165`:
```rust
//! # Where things live
//!
//! This file holds the [`ComAmazonawsSqsAmazonsqsState`] registry and the constants shared by
//! the queue model. The rest is one submodule per concern, all re-exported here:
//!
//! - `queue` and `message`: the [`Queue`] record and what it stores per message.
//! - `attributes`, `queue_rules`, `message_attributes`: what a request may supply.
//! - `redrive`, `message_move`: dead-letter links and message movement tasks.
//! - `address`, `receipt_handle`: how queues and receipt handles are named and read back.
//! - `limits`, `refusals`, `clock`, `digest`: bounds, error wording and small pure helpers.
```
A short map that lets a maintainer find the right file in a split that is otherwise only glob re-exports.

d40b5167e, `state/limits.rs:3-10`:
```rust
//! The in-memory store's own ceilings. The emulator authenticates nobody, so everything a
//! caller can make it hold is bounded. These are NOT fidelity claims about any published
//! quota: each is a ceiling generous enough that no realistic workflow meets it, so a caller
//! looping on a request cannot grow this process's memory without bound.

use super::*;

/// How many queues this emulator will hold; `CreateQueue` refuses past it.
pub const MAX_QUEUES: usize = 1_000;
```
A `// ----` banner section becomes a module whose rationale is its rustdoc, so all the bounds have one home.

## Worst hunks
none (the weakest choice is `use super::*;` at the top of every new file, e.g. 697c442a2 `state/digest.rs`,
which hides each module's real dependencies, though it follows the existing baseline modules).

## Silent edits check
For every commit I diffed the multiset of removed lines against added lines, ignoring indentation and blank
lines. Outside comments, the only differences are `mod`/`pub use` lines, `use super::*;`,
`impl ComAmazonawsSqsAmazonsqsState {` wrappers (850637adc, d3971d58f), and the visibility changes on
`epoch_seconds`, `queue_arn_fields`, `encoded_opaque_token`, `RECEIPT_HANDLE_FIELD_SEPARATOR`,
`REDRIVE_PERMISSION_MEMBER` and `SOURCE_QUEUE_ARNS_MEMBER`. I also spot-checked `queue_key`/`queue_key_from_arn`
(`state/address.rs:14,195`), `Queue::integer` (`state/queue.rs:281`) and the redrive impl block
(`state/redrive.rs:519`) against baseline: identical. Comment-only differences are the banners turned into
`//!` headers and the two dropped `///` docs on `mod message_attributes;` / `mod permissions;`, whose content
is already in each file's `//!`. No silent edits found.
