# Review of run 7
## Summary
The attempt split five cohesive groups out of the 4,877-line `state.rs` (message move tasks, queue address helpers, clock helpers, receipt handles, attribute/name/tag validation) into `state/*.rs` modules, bringing it to 3,769 lines, with a follow-up commit narrowing the visibility the splits had widened. It ran rustfmt over the whole crate in one isolated commit, rewrote `delivery_order` and `reapply_delay_seconds` as iterator chains, and renamed the `scratch_probe` test file. All moves are verbatim apart from dropped banner comments, added module docs and `pub(super)` on items siblings reach. It left alone the stringly domain types (`queue_key` still returns `(String, String)`, attributes still a string map), the `expect`-after-lookup sites, the op-file scaffold (`lift`/`store_unavailable`), the lint allows, the giant handlers in `ops/`, and comment volume. Tests and clippy output are identical to baseline (280 passed, same 7 clippy lines).

## Commit classification
| hash | subject | kind | pure move? |
|---|---|---|---|
| fa0445f65 | refactor: move message move task items out of state.rs | move | yes (plus pub(super) on 4 items, banner dropped) |
| a40715d1b | refactor: move queue key, ARN and url helpers into queue_address | move | yes (pub(super) on 2 items) |
| d2376f098 | refactor: move epoch and duration helpers into clock | move | yes (pub(super) on epoch_seconds) |
| f4014b00f | refactor: keep extracted helpers private unless a sibling needs them | other (visibility) | no |
| 63252522e | refactor: move receipt handle logic into its own module | move | yes (pub(super) on 2 items) |
| a54bc3d79 | refactor: move attribute, name and tag validation into its own module | move | yes |
| e57c9993e | style: apply rustfmt to the crate | fmt | n/a (formatting only, 78 files) |
| 5e58b732e | refactor: express delivery order and delay reapplication as iterator chains | rewrite | no |
| 2ebcedd1f | test: rename scratch_probe to say what it verifies | other (rename) | yes (100% similarity) |

## Scores
| dimension | score | evidence |
|---|---|---|
| Structure | +1 | fa0445f65..a54bc3d79: state.rs 4,877 -> 3,769 lines, five cohesive modules (`state/queue_address.rs:1`, `state/receipt_handle.rs`, `state/clock.rs:1`); f4014b00f keeps visibility minimal. Offset: new modules use `use super::*;` and `pub use x::*` (state.rs:205-213), glob-use metric 4.03 -> 4.53 |
| Duplication | 0 | No helper or handler boilerplate removed; `lift`/`store_unavailable` per op untouched |
| Error handling | 0 | `.expect(` rate unchanged (0.71); 5e58b732e drops one unreachable `let ... else { continue }` (old state.rs:1651) but that is negligible |
| Domain types | 0 | `queue_key` still `(String, String)` at `state/queue_address.rs:14`; no newtypes or enums added |
| Ownership | 0 | clone()/to_string() rates flat (8.42 -> 8.39, 6.68 -> 6.66, dilution only) |
| Comments | 0 | Comment share 43.4% -> 43.3%; banners removed (0.61 -> 0.51) but five module `//!` docs added; no narration pruned |
| Control flow | +1 | 5e58b732e `state.rs:1600-1608`, `:1644-1651`, `:1831-1836`: manual loops with repeated `len() >= max` guards become filter/take and cycle/skip/take chains |
| Lint suppressions | 0 | `#[allow(` count unchanged; clippy.txt identical; e57c9993e only reflows the allow list in `lib.rs:4-9` |
| Tests | 0 | Only rustfmt (e57c9993e) and a file rename (2ebcedd1f, `tests/all.rs`); no assertion changed |
| Commit hygiene | +1 | One kind of change per commit, bodies explain why; fmt isolated. Minus: f4014b00f is a fix-up of the three earlier move commits rather than folded in, and fmt lands mid-series |
| Behavior risk | 0 | Checked 5e58b732e: `cycle().skip(start).take(len)` equals the old `(start + offset) % len` walk for `start <= len` (start is `position + 1` or 0) and yields nothing for empty `runs`, as before; filter+take on non-FIFO keeps first `max` visible indices |

## Audit findings
| id | status | evidence |
|---|---|---|
| S-01 | untouched | |
| S-05 | untouched | |
| S-07 | untouched | |
| S-08 | untouched | |
| S-09 | untouched | `runs.iter().position` and `handles.retain` scans remain (state.rs:1642) |
| S-12/S-13 | untouched | |
| S-14/S-15 | untouched | |
| S-16/S-17 | untouched | |
| S-18 | partial | state.rs shrunk via module splits (fa0445f65..a54bc3d79) and `delivery_order` shortened (5e58b732e), but no giant handler in `ops/` split |
| S-19 | untouched | |
| S-20 | untouched | |
| S-21 | untouched | |

## Best hunks
5e58b732e, `src/state.rs:1644-1651`
```rust
        runs.iter()
            .cycle()
            .skip(start)
            .take(runs.len())
            .flat_map(|(_, run, _)| run)
            .copied()
            .take(max)
            .collect()
```
Replaces a 16-line nested loop with two duplicated cap checks and an unreachable `else continue` with a rotation that reads as what it is.

a40715d1b, `src/state/queue_address.rs:1-16`
```rust
//! Where a queue lives: the key it is stored under, its ARN and its url, and reading each
//! of them back into a name and region.

use super::*;

pub fn queue_key(region: &str, name: &str) -> (String, String) {
    (region.to_string(), name.to_string())
}
```
Key, ARN and url building and parsing now sit in one small file, moved verbatim, so the region-scoping rule can be read in one place.

## Worst hunks
fa0445f65 (and siblings), `src/state.rs:205-213` with `src/state/clock.rs:6`
```rust
pub use clock::*;
pub use message_move_task::*;
pub use queue_address::*;
pub use queue_attribute_validation::*;
pub use receipt_handle::*;
// each new module: use super::*;
```
The split modules import all of `state` by glob and are re-exported by glob, so the module boundaries do not show dependencies and the flat namespace is unchanged.

## Silent edits check
Compared removed vs added line multisets for every move commit (fa0445f65, a40715d1b, d2376f098, 63252522e, a54bc3d79): the only differences are dropped `// ----` banner comments, new SPDX/`//!` headers, `mod`/`pub use` lines, `use super::*;`, and `pub(super)` added to `NANOS_PER_SECOND`, `deliverable_by`, `in_progress`, `is_of`, `ENDPOINT_DNS_SUFFIX`, `queue_arn_fields`, `epoch_seconds`, `RECEIPT_HANDLE_FIELD_SEPARATOR`, `encoded_opaque_token` (two reverted in f4014b00f). Function bodies (e.g. `queue_key`, `queue_arn_fields`, `epoch_seconds`, `encoded_opaque_token`, `is_of`) are byte-identical to baseline. The rustfmt commit shows whitespace and line-wrap changes only in the src files sampled (build.rs, lib.rs, ops/mod.rs, service.rs).
