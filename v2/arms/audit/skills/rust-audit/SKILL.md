---
name: rust-audit
description: Find what to fix in existing Rust code. Use first for any request to clean up, refactor, review, tidy, simplify or improve existing Rust code, before reading the code in depth and before planning.
---

# Auditing existing Rust

A cleanup is judged by the defects it removes, not by how much code moved. Find the defects first.

## What to look for

Types
- identifiers, names and handles passed around as `String` or `&str`, and maps keyed by them;
- states, kinds, modes and codes kept as strings or integers and compared to literals or constants;
- `Option` or `bool` fields that only make sense together;
- settings kept as string maps and parsed again on every read.

Failures
- `expect` or `unwrap` right after a lookup or check that already proved the value is there;
- a failure turned into a default, an empty collection or `None`;
- a branch or re-validation for a case an earlier check already refused;
- a `Result` dropped with `let _ =` or `.ok()`.

Ownership and repeated work
- clones made only to satisfy the borrow checker or to build a lookup key;
- the same value looked up, parsed, sorted or scanned more than once per request;
- a scan of a collection inside a loop over the same collection.

Simplicity
- the same steps copied across handlers instead of living in one place;
- a decision computed in two places;
- `#[allow]` attributes that hide nothing, dead code, leftovers.

Comments and tests
- comments that restate the code or contradict it;
- tests that cannot fail: `is_ok()`, non-emptiness or substring checks, assertions behind a branch.

## How to work

1. Run the search commands from the "Review checklist" of each of these skills over the code:
   `rust-domain-types`, `rust-error-discipline`, `rust-ownership`, `rust-keep-it-simple`,
   `rust-comments`, `rust-test-quality`. Read the hits.
2. Write down the findings by category, with file and line, before changing anything.
3. Fix one category per commit, most severe first. Load the matching skill for how to fix.
4. Move code between files last, and only if it still helps after the fixes.
