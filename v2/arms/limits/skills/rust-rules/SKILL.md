---
name: rust-rules
description: Write, change or refactor Rust code. Use first, before writing or editing any Rust source in this project, whatever the task is (a new crate, a feature, a bug fix, a cleanup). It holds this project's Rust rules.
---

# Rust rules for this project

Follow these rules in all Rust code you write or change here.

Scope: when asked to add or change something, apply the rules to the code you add or touch and leave the rest alone. When asked to clean up, refactor or review existing code, apply them to that code.

## Types

- known values are compared as enum variants, not as strings;
- outside wire types, identifiers of different kinds are not the same primitive, and map keys are typed;
- types with a validity rule have no `pub` field, infallible `From` or plain `Deserialize`;
- `Option` and `bool` fields that change together are one enum;
- "not set" is `Option`, not `-1`, `0` or `""`;
- outside wire types, no field keeps its unit or format only in its name;
- data with a known shape is a struct, not `serde_json::Value` or `HashMap<String, String>`;
- no newtype is converted back to its primitive at every use.

But:

- Do not wrap free text, or values with no rule and no realistic mix-up. A newtype converted back to its primitive at every use (`.0`, `as_str()`, `into_inner()`) is a name, not safety.
- Do not enforce, in client code, constraints that only the server owns.
- Plain data may have public fields and no accessors. A type with an invariant has private fields and only the getters callers need, named after the field, without `get_` and without setter pairs. Do not mix `pub` and private fields on one type. Keep items private by default and use `pub(crate)` for internal sharing.
- When porting from another language, translate meaning, not spelling. Integer JSON numbers are `u64` or `i64`, not `f64`. JavaScript's `a || b` also replaces `""` and `0`, so `unwrap_or` alone changes behavior.

## Errors

- no failure becomes a default, an empty collection or `None` unless the spec defines that value;
- no `Result` is dropped with `let _ =`, `.ok();` or an unconditional `Ok`, except commented best-effort calls;
- nothing that input can trigger panics: no `unwrap`, `expect`, index or slice on input;
- every `expect` and `unreachable!` names the invariant, and no `todo!()` or `unimplemented!()` remains;
- `catch_unwind` appears only at a thread, task or FFI boundary;
- sizes and offsets that come from input use `try_from` and checked arithmetic;
- an error that crosses a layer says what was attempted, and is logged or returned, not both;
- no error type is `String`, `&'static str` or `()`, and no library function returns `anyhow::Result`;
- no `Option`, `Result` or branch exists for a state that cannot occur.

But:

- "No `unwrap`" never means "use a default". Replacing a panic with `unwrap_or_default()` trades a loud bug for a wrong answer; use `?`, or `expect` with the invariant.
- Do not invent error variants for states that only a bug can reach. Panic there.
- A bare `unwrap()` is fine in tests, on a poisoned `Mutex::lock()`, and when the check that guards it is on the adjacent line.
- Do not add context at every `?` inside one layer, and do not wrap an error in words it already says.
- Do not build an error enum that no caller matches on. In a binary, `anyhow` with context is enough.
- A deliberate second check at a trust boundary is not defensive code. Keep it.

## Ownership

- no `.clone()` was added only to make a borrow error go away, and no collection is cloned to iterate over it or cloned and then cleared;
- parameters that are only read are borrowed views, arguments that are stored are taken owned, and getters return borrows or iterators;
- no `String` is allocated to compare or look up, and no `Vec` is collected only to iterate, count or test;
- nothing is parsed, compiled, sorted or scanned per call or per item that could be done once;
- no buffer is created per iteration, no capacity comes straight from input, and nothing is formatted for a log line that may be off;
- no whole state is cloned out of a lock, and no guard is held during I/O or other slow work.

But:

- Keep a clone that is cheap, cold or clearer than the alternative. Do not thread lifetimes through structs to avoid one; own the data.
- Keep `&str` in a constructor when almost every caller only holds a borrow. `impl Into<String>` and `impl AsRef<str>` belong at published library entry points, not in internal code.
- A linear scan over a handful of items is fine and often faster than a map. Startup code, cold administrative paths and tests may recompute, and `format!` is fine for one-off strings, argument parsing and error messages.
- Do not add `Arc` without real sharing, and do not call `with_capacity` without a known size.
- Removing an accidental allocation or a rescan inside a loop needs no benchmark. A non-obvious optimization (`#[inline(always)]`, a custom hasher, a small-vector crate, interning, a hand-rolled loop) lands only with the benchmark that justifies it.

## Simplicity

- every new function, field, parameter, variant and trait method has a caller;
- every new trait has two implementations, and no new state repeats what existing state answers;
- no function has a suffixed twin, and no helper repeats one from the workspace, std or a dependency;
- no decision is computed in two places, and shared code does not branch on a concrete implementation;
- nothing exists only for an old shape that has no users;
- no value is checked and then unwrapped, and no flag variable feeds a second `if`;
- push, flag and accumulator loops are adapters, and chains with effects, `?` or early exits are loops;
- no `dbg!`, stray print, commented-out code or new `#[allow]` remains;
- every new dependency has a named need and only the features in use.

But:

- Keep a trait with a real second implementation, including a test fake the project uses. `with_x` builder methods that set one option are not twins.
- Keep the old function, format or column for a release when real users or stored data depend on it.
- Keep a single-use helper that names a concept or lets its body use `?` and `return`. A long linear function with one concern is fine, and so is one special case inline until a second appears.
- Do not add a dependency to save a few lines, and do not hand-write tricky parsing that a well-known crate already does.
- Keep a loop when the chain would change behavior: `any` and `find` stop early and skip effects the loop ran, `zip` stops at the shorter input where indexing would have panicked, and `collect::<Result<_, _>>()` keeps only the first error. Keep the index when it reaches neighbors or the loop changes the collection.
- Keep a `match` on a `Result` when a site recovers from the error or maps it to a different variant, and `if`/`else` when both branches carry similar weight.
- Keep `&mut` output for a caller-owned buffer reused across a hot loop, and `#[allow]` on generated, macro and cfg-dependent code.

## Comments and commits

- every added comment states a reason, an invariant or a contract, and none restates the line below it;
- no comment or doc mentions what the code used to do, the review or the plan, and none is made false by the change;
- added comments, docs and messages are plain ASCII, with no banners, step numbers or capitals for emphasis;
- public items say what the signature cannot, and no doc only rephrases the name;
- every `TODO` or `FIXME` names an action and an issue, and nothing is stubbed;
- every `SAFETY` comment names the check that makes the block sound;
- each commit has one purpose, `git diff` and `git diff -w` show the same lines, and the pull request text matches the diff.

But:

- Do not delete or shorten a comment that records a reason, an invariant, a workaround or a rejected alternative, however long it is. Removing it is a regression. When unsure, keep it.
- Leave existing comments alone unless your change makes them false. Rewording them is an off-topic diff. Comment density is the project's choice: do not add comments to reach a quota, and do not strip them to look terse.
- A marker that points forward (`FIXME(#412): ...`) and a note on why the obvious alternative was rejected are not history.
- Signal words (`NB:`, `Subtle:`, `SAFETY:`), acronyms in capitals and short worked examples are fine. One non-ASCII character in a string literal or a fixture is not the target.
- `todo!()` is fine in a local draft that is not presented as finished, and an explicit "unsupported" error with a tracking issue is honest.
- A large pull request is acceptable when each commit can be reviewed on its own. Cleanup is welcome as a separate pull request.

## Tests

- every test asserts a value, a count or an error variant, not only `is_ok()`, emptiness or a substring;
- no assertion sits behind `if let`, `unwrap_or`, `unwrap_or_default` or a loop that may not run;
- no expected value is computed by the code under test, and each regression test was seen failing without the fix;
- structured output is compared as a whole typed value or a reviewed snapshot;
- no fixed sleep, port or path, environment change, wall clock or random value decides a result;
- production code has no test-only branch;
- test names describe the scenario, and the inputs of one behavior share one table;
- no new helper repeats an existing one or hides the assertions.

But:

- Assert the key fragment of human prose such as an error message, not its full text. `contains` is the right check there.
- A multi-step end-to-end scenario is a good test when its name says so. Do not split it, and do not force tests with different bodies into one table or macro.
- A few repeated setup lines that keep a test readable on its own are fine. Do not extract a helper for them.
- Do not build a trait and a fake for something the test can run for real, such as a temp dir or a local socket.
- A sleep is fine when the timer itself is under test or the clock is paused. Comparing against a simpler reference implementation in a property test is a valid oracle.
