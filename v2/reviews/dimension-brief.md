# Blind dimension review brief

Several coding agents each produced a Rust crate for the same task. You compare ALL of them on ONE
dimension, so that the scores are calibrated against each other. You are told the phase `P` and the
dimension `D`.

## Where things are

- The crates: `~/code/wye/jrr-refactor-trial/v2/blind/P/<CODE>/` (each has `src/`, maybe `tests/`,
  `Cargo.toml`). The two-letter codes are random.
- The task they implement: `~/code/wye/jrr-refactor-trial/v2/task/SPEC.md` (and `SPEC-feature.md`
  when the phase is `p2`).
- If a directory named `BASE` exists, it is the crate every attempt started from. Score it like the
  others, and judge each attempt mainly on the code that differs from `BASE` (`diff -r` is allowed).

## Hard rules

- Read nothing outside `v2/blind/P/` and the two spec files. In particular not `v2/keys/`,
  `v2/runs/`, `v2/eval/`, `v2/logs/`, `v2/arms/`, `v2/PLAN.md`, `v2/STATE.md`, other phases, other
  reviewers' files, or anything in the parent directory.
- Do not run `cargo`. Do not modify the crates. The only files you write are
  `v2/reviews/P/D.md` and `v2/reviews/P/D.tsv`.
- You do not need to read every line of every crate. Read what your dimension needs, in every
  crate, with the same method for each (the same searches, the same kinds of files), so no crate
  gets a closer look than another. Say in your report what method you used.
- Every score needs two pieces of evidence as `CODE/file:line`, one for the best thing and one for
  the worst thing you saw on this dimension in that crate.

## Dimensions

Score 0 to 4: 0 = the problem is everywhere, 1 = common, 2 = mixed, 3 = rare lapses, 4 = consistently
good. Use the whole scale across the set; if two crates are equal, give them the same score.

- `types`: identifiers, names and handles of different kinds have their own types instead of bare
  `String`/`&str`/integers; closed sets (states, kinds, error codes, attribute names) are enums and
  are not compared as strings; input is parsed into typed values at the boundary and the typed
  value is passed inward; fields that only make sense together are one enum or struct; "not set"
  is `Option`, not a sentinel; no data with a known shape is kept as `serde_json::Value` or a string
  map past the boundary.
- `errors`: failures are not turned into defaults; no `Result` is dropped; nothing input can
  trigger panics (`unwrap`, `expect`, indexing, unchecked arithmetic or `as` casts on input); every
  remaining `expect` names an invariant; error types are typed, not `String`; no branch exists for
  a state that cannot occur; numeric input uses checked conversion.
- `ownership`: no clone that only quiets the borrow checker; read-only parameters are borrowed and
  stored ones are taken owned; no `String` allocated to compare or look up; nothing is parsed,
  sorted or scanned again per call or per item when once is enough; no collection is rebuilt or
  scanned inside a loop over itself.
- `simplicity`: no abstraction, trait, generic, builder, field or helper without a second user; no
  decision computed in two places; no copied blocks across handlers; branches and loops in their
  direct form; no dead code, leftover debugging, or `#[allow]`; module layout matches the size of
  the problem.
- `comments`: comments state reasons, invariants or contracts and none restates the code;
  no banners, step numbers, history or plan talk; doc comments say what the signature cannot;
  comment volume is proportionate.
- `tests`: tests assert exact values, counts or error variants rather than `is_ok()`, non-emptiness
  or substrings; no assertion sits behind a branch that may not run; expected values do not come
  from the code under test; cases are deterministic and independent; names describe the scenario;
  the important behaviors of the spec are covered.
- `overall`: no checklist. Which of these crates would you rather inherit and maintain for two
  years? Judge readability, how safely a change can be made, and how much you would have to learn
  before trusting it.

## Output

`v2/reviews/P/D.tsv`: one line per crate, `CODE<TAB>score<TAB>rank` (rank 1 = best, ties share a
rank), no header.

`v2/reviews/P/D.md`, at most 150 lines:

```
# P / D
## Method
(what you read in each crate and how you kept it even)
## Scores
(table: code | score | rank | best evidence | worst evidence | one-line reason)
## What separates the top from the bottom
(3 to 6 bullets with evidence)
## Patterns seen in most crates
(what nearly all of them do well or badly on this dimension)
```

Final message back: the TSV content, then one sentence. Nothing else.
