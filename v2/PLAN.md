# Trial v2: under which circumstances do the just-right-rust skills add value

Follows `../TRIAL-LOG.md` (v1: 14 refactor runs on the aws-emulator SQS crate). v1 result: narrow
skills are never loaded on a broad request, a router skill loads them every time, loading alone
showed no benefit, and three short "what to look for" lines produced the only domain-type change.

v2 varies the things v1 held fixed: task type, delivery mechanism, model. Everything runs headless
(`claude -p`), Sonnet 5.5 unless stated, plain prompts (no "use the skills" line).

## What v2 changes in the harness

- Small standalone crate ("miniq", see `task/`) instead of the 19.6k-line SQS crate: builds in
  seconds, a run takes minutes, so n=3 per arm is affordable.
- Hidden black-box acceptance tests (`task/acceptance.py`) give a behavior check the agent never
  sees. The interface is newline-delimited JSON on stdin/stdout, so internal modelling is free.
- Agent work directories live in `/Users/Shared/jrr-work/<run>` (outside `~`), to keep
  `~/.claude/CLAUDE.md` out of the runs. Results are copied back to `v2/runs/<run>`.
- No symposium in the loop: plugin directories are passed with `--plugin-dir` (symposium delivery
  was validated in v1 and is not the question here).
- Batches run under `caffeinate` and a per-run timeout (v1 lost a batch to machine sleep).
- Blind review is per dimension across all runs of a phase (one reviewer reads every crate under a
  shuffled code), which fixes the v1 calibration problem (one reviewer per run).

## Arms (delivery mechanisms), all in `arms/`

| Arm | What the agent gets |
|---|---|
| `none` | no plugin |
| `skills` | the 8 skills of branch `ai-patterns-skills`, as they are |
| `router` | `skills` + an entry skill whose description covers writing or changing any Rust, and whose body only says to load the others (no hints) |
| `audit` | `skills` + `rust-audit`: an entry skill for existing code with a short generic list of what to look for and the skills' own search commands (refactor phases only) |
| `context` | no plugin; the six "Review checklist" bullet lists pasted into the project's `CLAUDE.md` (same content, always in context) |
| `hook` | `skills` + a Stop hook that runs a detector (clippy with the research lint set plus the skills' searches) once and hands the findings back before the agent may finish |

## Phases

| Phase | Task | Arms | n | Question |
|---|---|---|---|---|
| P1 | Greenfield: build miniq from `SPEC.md` | none, skills, router, context, hook | 3 | Do the rules help when writing new code, and through which delivery? |
| P2 | Feature: add `SPEC-feature.md` to a fixed AI-written baseline (a P1 `none` crate) | none + best three of P1 | 3 | Does new code in an existing codebase follow the rules or the surrounding style? |
| P3 | Refactor: "clean it up" on the same baseline | none, skills, router, audit, context | 3 | v1 again, cheap, with behavior checked by hidden tests |
| P4 | Greenfield, other models (Haiku 5.5, Opus 5.5) | none + best of P1 | 2 | Does the effect depend on the model? |
| P5 | Refactor of the SQS crate with `audit` | audit | 2 | Does the P3 idea transfer to the large v1 crate? |

Decision rules, fixed before running: the P2/P3 baseline is the P1 `none` run with the median
blind total among those that pass acceptance; "best of P1" is by blind total, ties by acceptance.

## Measures per run

- Acceptance pass rate (hidden scenarios), build ok, own tests pass.
- Detectors (`detect.py`): clippy lint-set count per KLOC, newtypes, enums, id-like `String`/`&str`
  fields and params, string comparisons to literals, `unwrap`/`expect`/`unwrap_or*` outside tests,
  `clone()`, `#[allow]`, comment share, tests and assertions.
- Transcript: skills invoked and when, turns, minutes, cost.
- Blind dimension scores 0 to 4: domain types, error discipline, ownership, simplicity, comments,
  test quality. For P2/P3 the baseline is included as a labelled reference.

## Layout

```
v2/PLAN.md  STATE.md  arms/  task/  bin/{run2.sh,eval2.sh,batch.sh,detect.py,blind.py}
v2/runs/<run>/   v2/eval/<run>/   v2/logs/<run>.jsonl   v2/blind/<phase>/   v2/reviews/<phase>/
v2/keys/         (run-to-arm and blind-code mappings; reviewers must not read)
```
