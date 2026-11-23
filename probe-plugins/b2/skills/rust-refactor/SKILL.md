---
name: rust-refactor
description: Refactor or clean up existing Rust code so it is idiomatic and easy to maintain. Use first for any request to clean up, tidy, refactor, simplify, modernize or improve the readability of Rust code, before reading the code in depth. It says what to look for and which other rust-* skills to load.
---

# Refactoring existing Rust

Moving code between files is the smallest part of a cleanup. Before planning the work, load each of
these skills and run its review checklist against the code you were asked to clean up:

- `rust-domain-types`: identifiers, states and configuration kept as strings, bools or string maps.
- `rust-error-discipline`: silent defaults, discarded results, `expect` after a prior lookup,
  unreachable defensive branches.
- `rust-ownership`: needless clones, owned lookup keys, repeated scans, long lock scopes.
- `rust-keep-it-simple`: duplicated logic, dead code, convoluted branches.
- `rust-comments`: comments that restate or contradict the code, commit messages.
- `rust-test-quality`: tests that cannot fail.

Plan the cleanup from what those checklists find, then split files if it still helps.
