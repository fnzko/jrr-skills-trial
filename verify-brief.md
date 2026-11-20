# Adversarial verification brief

A reviewer scored one refactor attempt (run name `N`) of a Rust crate and wrote
`~/code/wye/jrr-refactor-trial/eval/N/review.md`. Assume the review is wrong somewhere: it may cite
evidence that does not exist, call an edit a pure move, score a cosmetic change as an improvement, or
miss a real change. Your job is to find those errors. You do not write a competing review.

## Where things are

- Trial root: `~/code/wye/jrr-refactor-trial`
- The attempt: git worktree `runs/N`, commits `baseline..HEAD`
- Crate: `services/service-com-amazonaws-sqs-amazonsqs`
- The rubric the reviewer followed: `review-brief.md` (read it first)
- Mechanical results: `eval/N/*.txt` and `eval/baseline/*.txt` (not `transcript.txt`)
- Audit findings referenced by the review:
  `~/code/wye/research/rust-idiomatic-ai-patterns-2026-11-16/reports/Auditoria aws-emulator core.md`

## Hard rules

- Do NOT read `mapping.tsv`, anything under `logs/`, any `eval/*/transcript.txt`, any `.claude/`
  directory, `homes/`, `prompt*.txt`, other runs' worktrees, or other runs' reviews.
- Do not run `cargo`. Do not modify the worktree. The only file you write is `eval/N/verify.md`.

## What to check

1. Every non-zero score: open the cited commit and `file:line`. Does the evidence exist, and does it
   support that score under the rubric? Verdict per score: `confirmed`, `too high`, `too low`,
   `no evidence`.
2. Every zero score for Duplication, Error handling, Domain types, Ownership, Comments, Control flow,
   Lint suppressions and Tests: look for a change the reviewer missed. Method: for each commit the
   review classifies as a pure move, diff the moved bodies against baseline
   (`git show --color-moved=zebra --color-moved-ws=allow-indentation-change <hash>` and look for
   lines not marked as moved); for the other commits read the full diff. Also compare counts in
   `eval/N/metrics.txt` and `eval/N/clippy.txt` with baseline. Verdict per dimension:
   `zero confirmed` or `missed: <what, commit, file:line>`.
3. Behavior risk: try to find one hunk that can change observable behavior (reordered side effects,
   changed visibility that alters name resolution, altered constants, changed error text, dropped
   branch). Verdict: `none found` or the hunk.
4. Each audit status that is not `untouched`: check the cited evidence against the finding's text.
   Verdict: `confirmed` or `overstated`.
5. The "Best hunks" and "Worst hunks": do they exist as quoted at that location?

## Output

Write `eval/N/verify.md`, in English, at most 60 lines:

```
# Verification of review N
## Score verdicts
(table: dimension | reviewer score | verdict | corrected score | evidence)
## Missed changes
(list, or "none")
## Behavior risk
## Audit status verdicts
## Hunks check
```

Final message back: one line with the 11 corrected scores in rubric order, one line listing only the
scores you changed and why (or "no changes"), nothing else.
