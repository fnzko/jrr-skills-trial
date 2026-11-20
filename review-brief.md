# Blind review brief

You are reviewing one refactor attempt of a Rust crate. Several attempts exist; each was made by a
coding agent given the same short request ("clean this crate up so it reads like Rust an experienced
maintainer would be happy to own, do not change behavior, small commits"). You review exactly one,
identified by its run name `N`. Your review is compared against reviews of the other attempts, so
follow the rubric and format exactly and score against the baseline, not against an ideal.

## Where things are

- Trial root: `~/code/wye/jrr-refactor-trial`
- The attempt: git worktree `runs/N`, commits `baseline..HEAD`
- Crate under review: `services/service-com-amazonaws-sqs-amazonsqs` (only this crate matters)
- Mechanical results already computed, read them instead of building: `eval/N/commits.txt`,
  `eval/N/diffstat.txt`, `eval/N/test.txt`, `eval/N/clippy.txt`, `eval/N/metrics.txt`, and the same
  files under `eval/baseline/`
- Prior audit of the untouched code:
  `~/code/wye/research/rust-idiomatic-ai-patterns-2026-11-16/reports/Auditoria aws-emulator core.md`
  Read only these findings: S-01, S-05, S-07, S-08, S-09, S-12/S-13, S-14/S-15, S-16/S-17, S-18,
  S-19, S-20, S-21 (sections 4.1 to 4.5). Their SQS exemplars point at baseline line numbers.

## Hard rules

- Stay blind. Do NOT read `mapping.tsv`, anything under `logs/`, any `eval/*/transcript.txt`, any
  `.claude/` directory, `homes/`, `prompt*.txt`, other runs' worktrees, or other runs' `review.md`.
- Do not run `cargo` (no build, test, clippy). Do not modify the worktree. Read-only git and file
  reads only. The only file you write is `eval/N/review.md`.
- Every score other than 0 needs evidence: a commit hash plus `file:line` in the HEAD tree (or in the
  diff). No evidence, score 0.
- Most of the diff is code moved between files. Find out which commits are pure moves
  (`git show --stat`, `git show --color-moved=zebra`, comparing moved bodies) and spend your time on
  the commits and hunks that change code. A move that also edits the moved code counts as a change:
  check a sample of moved functions against the baseline for silent edits.

## Rubric

Score each dimension relative to baseline: `-1` worse, `0` unchanged or negligible, `+1` some real
improvement, `+2` clear and broad improvement. Judge what a maintainer would feel reading HEAD.

1. Structure: module layout, file sizes, cohesion, visibility (`pub(crate)` creep counts against).
2. Duplication: repeated handler boilerplate or helpers actually removed, not just relocated.
3. Error handling: `expect`/`unwrap` after a prior lookup, stringly errors, swallowed results,
   unreachable defensive branches.
4. Domain types: identifiers and closed sets as types instead of `String`/`(String, String)`/bools,
   coupled `Option` fields, string maps re-parsed on read.
5. Ownership: needless `clone()`/`to_string()`, owned keys built per lookup, borrows where possible.
6. Comments: narration that restates code, stale or contradicting comments, doc comments that carry
   contract or rationale. Count comment volume change from `metrics.txt`.
7. Control flow: let-else, early returns, iterator chains where clearer, shorter functions, giant
   handlers split with meaningful names.
8. Lint suppressions: `#[allow(...)]` removed, narrowed, or given a reason; compare `clippy.txt`.
9. Tests: any test changed, added or weakened. Unchanged tests score 0.
10. Commit hygiene: one kind of change per commit, message says why, history reviewable in order.
11. Behavior risk (negative only: `0` none found, `-1` plausible change, `-2` definite change):
    anything that can alter observable behavior despite green tests. Name the hunk.

Then, for each audit finding listed above: `addressed`, `partial`, or `untouched`, with one line of
evidence when not `untouched`.

## Output

Write `eval/N/review.md`, in English, at most 120 lines, with exactly these sections:

```
# Review of run N
## Summary
(4 to 6 sentences: what the attempt did, what it left alone)
## Commit classification
(table: hash | subject | kind: move / dedupe / rewrite / fmt / other | pure move? yes/no)
## Scores
(table: dimension | score | evidence)
## Audit findings
(table: id | status | evidence)
## Best hunks
(2 hunks: commit, file:line, <= 15 lines of code each, one sentence on why)
## Worst hunks
(up to 2: same format; "none" is allowed)
## Silent edits check
(which moved functions you compared with baseline, and what you found)
```

Your final message back: the 11 scores on one line in rubric order, the audit statuses on one line,
and one sentence of overall verdict. Nothing else.
