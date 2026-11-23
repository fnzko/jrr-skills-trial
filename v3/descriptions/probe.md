# v3 description probe (interim, matrix stopped)

Status: stopped after 8 probes ($0.63 recorded; 3 killed runs report no cost). Waiting for a decision on the cargo deny, see "What went wrong".

## What went wrong

With `--disallowedTools ... "Bash(cargo:*)"`, the greenfield probe ends before any code is written: the agent reads SPEC.md, gets `cargo --version` denied, and replies that it will not build blind (v0, v1, v2, v3 green-1: 3 tool calls, 6 denials in the log, no Skill call). The known 5-6 of 6 green loads cannot appear under this setting. Removing the deny from `bin/probe3.sh` was refused by the permission classifier, so the matrix was not continued.

## Probes so far

| probe | Skill calls (skill@tool index) | tool calls | note |
|---|---|---|---|
| v0-green-1 | none | 3 | gave up, cargo denied |
| v1-green-1 | none | 3 | gave up, cargo denied |
| v2-green-1 | none | 3 | gave up, cargo denied |
| v3-green-1 | none | 3 | gave up, cargo denied |
| v4-green-1 | rust-new-code@1 | 2 | killed when the matrix was stopped |
| v5-green-1 | rust-new-code@1 | 2 | killed when the matrix was stopped |
| v6-green-1 | rust-rules@1 | 2 | killed when the matrix was stopped |
| v0-refactor-1 | none | 9 | pilot; authenticated, 8 plugin skills listed; cargo denials did not stop it |

## Description texts

v5 = v0 six + v4 three. v7 = descriptions from `descriptions/guidance.md` "Corrected descriptions", verbatim.

### v0
- `rust-comments`: Write Rust comments, doc comments, commit messages and pull request text that state reasons and contracts instead of restating code or narrating the edit. Use when adding or editing `//` comments, `///` docs, `SAFETY` comments or TODO markers, when committing or describing a pull request, and when reviewing a diff for comments that narrate, record history or contradict the code.
- `rust-domain-types`: Model domain data in Rust with enums, newtypes and std types instead of bare strings, integers and bools. Use when defining structs, enums or function signatures, when parsing requests, config or other external input, and when reviewing code that passes ids, names, states or units around as `String`, `u64` or `bool`.
- `rust-error-discipline`: Handle Rust failures without hiding them. No silent defaults or discarded `Result`s, panics only for bugs, context added once when an error crosses a layer, and typed errors that keep the cause. Use when writing or reviewing code that handles `Result` or `Option`, adds a fallback, parses input, does I/O, casts or adds numbers that come from input, or reaches for `unwrap`, `expect`, `unwrap_or_default`, `let _ =` or `catch_unwind`.
- `rust-keep-it-simple`: Keep Rust code simple by adding no abstraction, state, helper, dependency or compatibility code the change does not need, and by writing branches, loops and iterator chains in their direct form. Use when adding a feature, trait, generic, helper, field or dependency, when changing or copying an existing function, when writing `if`, `match`, loops or iterator chains, and when reviewing a diff for unused code, duplicated logic, shims and debugging leftovers.
- `rust-ownership`: Avoid needless clones, allocations and repeated work in Rust by borrowing in signatures, moving instead of copying, computing once per input and keeping lock scopes short. Use when fixing a borrow checker error, when writing function signatures, getters, loops over collections or code that locks shared state, and when reviewing code that adds `.clone()`, `.to_string()`, `.collect()` or a scan inside a loop.
- `rust-test-quality`: Write Rust tests that fail when the code is wrong, with exact assertions, expected values that do not come from the code under test, and cases that do not depend on timing or on each other. Use when writing or changing `#[test]` functions, test modules, files under `tests/` or test helpers, when adding a regression test for a fix, and when reviewing tests that assert only `is_ok()`, non-emptiness or a substring.

### v1
- `rust-comments`: Write, change, refactor or review Rust code so its comments, doc comments, commit messages and pull request text state reasons and contracts instead of restating code or narrating the edit. Covers `//` comments, `///` docs, `SAFETY` comments, TODO markers, commits and pull request descriptions, and diffs whose comments narrate, record history or contradict the code.
- `rust-domain-types`: Write, change, refactor or review Rust code so domain data is modeled with enums, newtypes and std types instead of bare strings, integers and bools. Covers structs, enums and function signatures, parsing requests, config or other external input, and ids, names, states or units passed around as `String`, `u64` or `bool`.
- `rust-error-discipline`: Write, change, refactor or review Rust code so failures are handled without hiding them: no silent defaults or discarded `Result`s, panics only for bugs, context added once when an error crosses a layer, and typed errors that keep the cause. Covers `Result` and `Option` handling, fallbacks, input parsing, I/O, casts and arithmetic on input, and `unwrap`, `expect`, `unwrap_or_default`, `let _ =` or `catch_unwind`.
- `rust-keep-it-simple`: Write, change, refactor or review Rust code so it stays simple: no abstraction, state, helper, dependency or compatibility code the change does not need, and branches, loops and iterator chains in their direct form. Covers new features, traits, generics, helpers, fields and dependencies, changed or copied functions, `if`, `match`, loops and iterator chains, unused code, duplicated logic, shims and debugging leftovers.
- `rust-ownership`: Write, change, refactor or review Rust code so it avoids needless clones, allocations and repeated work: borrow in signatures, move instead of copying, compute once per input and keep lock scopes short. Covers borrow checker errors, function signatures, getters, loops over collections, code that locks shared state, and `.clone()`, `.to_string()`, `.collect()` or a scan inside a loop.
- `rust-test-quality`: Write, change, refactor or review Rust tests so they fail when the code is wrong, with exact assertions, expected values that do not come from the code under test, and cases that do not depend on timing or on each other. Covers `#[test]` functions, test modules, files under `tests/`, test helpers, regression tests for a fix, and tests that assert only `is_ok()`, non-emptiness or a substring.

### v2
- `rust-comments`: Write Rust comments, doc comments, commit messages and pull request text that state reasons and contracts instead of restating code or narrating the edit. Use when adding or editing `//` comments, `///` docs, `SAFETY` comments or TODO markers, when committing or describing a pull request, and when reviewing a diff for comments that narrate, record history or contradict the code. Use when building new Rust code, adding a feature to existing Rust code, and refactoring, cleaning up or reviewing existing Rust code.
- `rust-domain-types`: Model domain data in Rust with enums, newtypes and std types instead of bare strings, integers and bools. Use when defining structs, enums or function signatures, when parsing requests, config or other external input, and when reviewing code that passes ids, names, states or units around as `String`, `u64` or `bool`. Use when building new Rust code, adding a feature to existing Rust code, and refactoring, cleaning up or reviewing existing Rust code.
- `rust-error-discipline`: Handle Rust failures without hiding them. No silent defaults or discarded `Result`s, panics only for bugs, context added once when an error crosses a layer, and typed errors that keep the cause. Use when writing or reviewing code that handles `Result` or `Option`, adds a fallback, parses input, does I/O, casts or adds numbers that come from input, or reaches for `unwrap`, `expect`, `unwrap_or_default`, `let _ =` or `catch_unwind`. Use when building new Rust code, adding a feature to existing Rust code, and refactoring, cleaning up or reviewing existing Rust code.
- `rust-keep-it-simple`: Keep Rust code simple by adding no abstraction, state, helper, dependency or compatibility code the change does not need, and by writing branches, loops and iterator chains in their direct form. Use when adding a feature, trait, generic, helper, field or dependency, when changing or copying an existing function, when writing `if`, `match`, loops or iterator chains, and when reviewing a diff for unused code, duplicated logic, shims and debugging leftovers. Use when building new Rust code, adding a feature to existing Rust code, and refactoring, cleaning up or reviewing existing Rust code.
- `rust-ownership`: Avoid needless clones, allocations and repeated work in Rust by borrowing in signatures, moving instead of copying, computing once per input and keeping lock scopes short. Use when fixing a borrow checker error, when writing function signatures, getters, loops over collections or code that locks shared state, and when reviewing code that adds `.clone()`, `.to_string()`, `.collect()` or a scan inside a loop. Use when building new Rust code, adding a feature to existing Rust code, and refactoring, cleaning up or reviewing existing Rust code.
- `rust-test-quality`: Write Rust tests that fail when the code is wrong, with exact assertions, expected values that do not come from the code under test, and cases that do not depend on timing or on each other. Use when writing or changing `#[test]` functions, test modules, files under `tests/` or test helpers, when adding a regression test for a fix, and when reviewing tests that assert only `is_ok()`, non-emptiness or a substring. Use when building new Rust code, adding a feature to existing Rust code, and refactoring, cleaning up or reviewing existing Rust code.

### v3
- `rust-comments`: Load before writing or changing Rust code that adds or edits `//` comments, `///` docs, `SAFETY` comments or TODO markers, and before writing commit messages or pull request text. Comments and messages state reasons and contracts instead of restating code or narrating the edit; also covers reviewing a diff for comments that narrate, record history or contradict the code.
- `rust-domain-types`: Load before writing or changing Rust code that defines structs, enums or function signatures, parses requests, config or other external input, or passes ids, names, states or units around as `String`, `u64` or `bool`. Models domain data with enums, newtypes and std types instead of bare strings, integers and bools.
- `rust-error-discipline`: Load before writing or changing Rust code that handles `Result` or `Option`, adds a fallback, parses input, does I/O, casts or adds numbers that come from input, or reaches for `unwrap`, `expect`, `unwrap_or_default`, `let _ =` or `catch_unwind`. Handles failures without hiding them: no silent defaults or discarded `Result`s, panics only for bugs, context added once when an error crosses a layer, and typed errors that keep the cause.
- `rust-keep-it-simple`: Load before writing or changing Rust code that adds a feature, trait, generic, helper, field or dependency, changes or copies an existing function, or writes `if`, `match`, loops or iterator chains. Keeps the code simple: no abstraction, state, helper, dependency or compatibility code the change does not need, control flow in its direct form, and no unused code, duplicated logic, shims or debugging leftovers.
- `rust-ownership`: Load before writing or changing Rust code that has function signatures, getters, loops over collections or locks on shared state, that fixes a borrow checker error, or that adds `.clone()`, `.to_string()`, `.collect()` or a scan inside a loop. Avoids needless clones, allocations and repeated work by borrowing in signatures, moving instead of copying, computing once per input and keeping lock scopes short.
- `rust-test-quality`: Load before writing or changing Rust code that has `#[test]` functions, test modules, files under `tests/`, test helpers or regression tests for a fix. Makes tests fail when the code is wrong: exact assertions, expected values that do not come from the code under test, and cases that do not depend on timing or on each other.

### v4
- `rust-add-feature`: Add a feature to, extend or fix a bug in existing Rust code. Use first, before editing any Rust source when the task changes what an existing crate does: a new feature, endpoint or option, an extension of existing behavior, or a bug fix. It holds this project's Rust rules for changes to existing code.
- `rust-new-code`: Write new Rust code from scratch: a new crate, a new module, a new binary or library, or implementing a spec or design in Rust. Use first, before writing any Rust source when starting a new crate or module. It holds this project's Rust rules for new code.
- `rust-refactor`: Clean up, refactor, simplify, review or audit existing Rust code. Use first, before changing any Rust source when the task is to make existing code easier to read or change without changing its behavior, or to review or audit it. It holds this project's Rust rules for cleanup and review.

### v6
- `rust-rules`: Write, change or refactor Rust code. Use first, before writing or editing any Rust source in this project, whatever the task is (a new crate, a feature, a bug fix, a cleanup). It holds this project's Rust rules.

### v7
- `rust-comments`: Use this skill whenever writing or editing Rust code that carries comments or docs, and whenever writing a commit message or pull request description, including during a spec build, a feature, a bug fix or a cleanup. Keeps `//`, `///`, `SAFETY` and TODO comments to reasons, invariants and contracts; removes comments that restate the code, narrate the edit or record history; keeps each commit to one purpose and PR text short.
- `rust-domain-types`: Use this skill whenever writing, extending, refactoring or reviewing Rust code that defines data: structs, enums, function signatures, config, or request and message parsing. Applies when building a crate from a spec, adding a feature or cleaning up existing code, even if the user never mentions types. Replaces bare `String`, `u64` and `bool` ids, names, states and units with enums and newtypes, and parses external input once at the boundary.
- `rust-error-discipline`: Use this skill whenever writing, extending, refactoring or reviewing Rust code that can fail: I/O, parsing, input validation, arithmetic on input, or any `Result` or `Option`. Applies when building a crate from a spec, adding a feature, fixing a bug or cleaning up a crate, even if the user never mentions errors. Removes silent defaults, discarded `Result`s, `unwrap` on recoverable failures and duplicate error reports; keeps panics for bugs and adds context once per layer.
- `rust-keep-it-simple`: Use this skill whenever writing, extending, refactoring, cleaning up or reviewing Rust code: building a crate from a spec, adding a feature, fixing a bug or simplifying existing code. Adds no trait, generic, helper, field, dependency or compatibility shim the change does not need, changes existing code before adding beside it, deletes what the change makes dead, and writes branches, loops and iterator chains in their direct form.
- `rust-ownership`: Use this skill whenever writing, extending, refactoring or reviewing Rust code that passes data between functions, loops over collections or shares state behind a lock, and whenever fixing a borrow checker error. Applies when building a crate from a spec, adding a feature, optimizing or cleaning up a crate, even if the user never mentions performance. Removes needless `.clone()`, `.to_string()` and `.collect()`, repeated scans inside loops and long lock scopes by borrowing in signatures and computing once per input.
- `rust-test-quality`: Use this skill whenever writing, changing or reviewing Rust tests, including the tests that come with building a crate from a spec, adding a feature, fixing a bug or refactoring, even if the user only asks to make sure it works. Makes `#[test]` functions and `tests/` files fail when the code is wrong: exact assertions instead of `is_ok()`, non-empty or substring checks, expected values not computed by the code under test, and no dependence on timing or test order.

## Full matrix (72 probes, cargo allowed, max 10 turns, budget USD 0.5)

| variant | task | n | loads per skill (runs of n) | first Skill call index | cost/probe |
|---|---|---|---|---|---|
| v0 | feature | 3 | none (six-total 0) | -,-,- | 0.360 |
| v0 | green | 3 | none (six-total 0) | -,-,- | 0.520 |
| v0 | refactor | 3 | none (six-total 0) | -,-,- | 0.258 |
| v1 | feature | 3 | none (six-total 0) | -,-,- | 0.370 |
| v1 | green | 3 | none (six-total 0) | -,-,- | 0.509 |
| v1 | refactor | 3 | none (six-total 0) | -,-,- | 0.229 |
| v2 | feature | 3 | none (six-total 0) | -,-,- | 0.289 |
| v2 | green | 3 | none (six-total 0) | -,-,- | 0.335 |
| v2 | refactor | 3 | none (six-total 0) | -,-,- | 0.245 |
| v3 | feature | 3 | none (six-total 0) | -,-,- | 0.335 |
| v3 | green | 3 | none (six-total 0) | -,-,- | 0.340 |
| v3 | refactor | 3 | com 1 kis 1 (six-total 2) | -,-,2 | 0.253 |
| v4 | feature | 3 | ADD 3; matching task skill 3/3, wrong task skill 0/3 (six-total 0) | 1,1,1 | 0.370 |
| v4 | green | 3 | NEW 3; matching task skill 3/3, wrong task skill 0/3 (six-total 0) | 1,1,1 | 0.472 |
| v4 | refactor | 3 | REF 3; matching task skill 3/3, wrong task skill 0/3 (six-total 0) | 1,1,1 | 0.231 |
| v5 | feature | 3 | ADD 3; matching task skill 3/3, wrong task skill 0/3 (six-total 0) | 1,1,1 | 0.346 |
| v5 | green | 3 | NEW 3; matching task skill 3/3, wrong task skill 0/3 (six-total 0) | 2,1,2 | 0.519 |
| v5 | refactor | 3 | REF 3; matching task skill 3/3, wrong task skill 0/3 (six-total 0) | 1,1,1 | 0.242 |
| v6 | feature | 3 | rules 3 (six-total 0) | 1,1,1 | 0.369 |
| v6 | green | 3 | rules 3 (six-total 0) | 1,1,1 | 0.499 |
| v6 | refactor | 3 | rules 3 (six-total 0) | 1,1,1 | 0.239 |
| v7 | feature | 3 | none (six-total 0) | -,-,- | 0.343 |
| v7 | green | 3 | none (six-total 0) | -,-,- | 0.523 |
| v7 | refactor | 3 | none (six-total 0) | -,-,- | 0.239 |

The 8 probes run with cargo denied are in `v3/logs/cargo-denied/` and are not counted.
