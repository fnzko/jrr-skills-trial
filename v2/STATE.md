# v2 state (append newest at the bottom)

## 2026-11-20

- Plan written (`PLAN.md`). Arms built in `arms/` (skills, router, audit, context, hook). Prompts in
  `prompts/`. Scripts in `bin/` (run2.sh, eval2.sh, batch.sh, detect.py, table.py, blind.py).
- Verified: a work dir under `/Users/Shared/jrr-work/` does not load `~/.claude/CLAUDE.md`.
- Verified: the Stop hook of the `hook` arm fires once, hands findings back, the agent answers them
  (`/Users/Shared/jrr-work/hooktest/log.jsonl`).
- `rg` is a shell function here, not a binary, so the hook script uses `grep -rnE`.
- P5 started: v1 runs `14` and `15` (SQS crate, `PLUGIN_DIR=v2/arms/audit`, plain prompt), logs in
  `../logs/14.jsonl`, `../logs/15.jsonl`; evaluate with `../eval.sh`, review with `../review-brief.md`.
- Task fixture (`task/`: SPEC.md, SPEC-feature.md, acceptance.py, reference/) being authored by a
  subagent.

Next: smoke one greenfield run, then P1 batch (15 runs: p1-01..p1-15, arms shuffled, key in
`keys/runs.tsv`), blind dimension reviews, pick baseline, P2 + P3, P4, synthesis.

### P5 results (SQS crate, audit skill, plain prompt)

| Run | Turns | Min | USD | Commits | Diff | clippy (base 360) | expect/KLOC (base 0.71) | Status enum | Blind sum |
|---|---|---|---|---|---|---|---|---|---|
| 14 | 35 | 9.8 | 0.66 | 5 | +75/-238 | 264 | 0.31 | no | 7 (reviewed, not verified) |
| 15 | 44 | 12.0 | 0.86 | 11 | +318/-434 | 166 | 0.15 | yes | pending |

Both loaded only `rust-audit` (none of the six guides), did almost no file moving, and finished in
10 to 12 minutes. v1 for comparison: best sums 11, 9, 9 took 100 to 130 turns.
Run 14 scores: Structure 0, Dup +1, Err +1, Types 0, Own +1, Comments 0, Flow +1, Lints +1, Tests 0, Hygiene +2.
Run 15 blind scores: Structure +1, Dup +1, Err +1, Types +1, Own +1, Comments 0, Flow +1, Lints +2, Tests 0, Hygiene +1 = 9 (reviewed, not verified). S-12/S-13 and S-16/S-17 partial in both runs.

### Task fixture ready (`task/`)

SPEC.md 220 lines, SPEC-feature.md 79 lines, acceptance.py: 58 base scenarios, 80 with `--feature`
(a base crate fails feature mode and the reverse; no flag for P1 and P3, `--feature` for P2).
Reference passes 80/80; 53 mutations all caught. Three scenarios feed 100000+ lines with a 10 s
timeout, so an implementation that rescans per command fails them.

### P1 launched

`keys/p1.list` (seed 20261120): router 01 09 15; hook 02 11 14; none 03 04 07; skills 05 08 12;
context 06 10 13. Background batch, 5 at a time, under caffeinate. p1-01 (router) loaded all seven
skills as its first actions.
After it finishes: `python3 -I bin/table.py p1`, then `bin/blind.py p1 p1-01 ... p1-15` and six
dimension reviewers (brief to write: `reviews/dimension-brief.md`).

### P1 mechanical results (greenfield, Sonnet 5.5, 15 runs, all 58/58 acceptance)

First attempt lost the six `none`/`context` runs to a script bug (empty array under `set -u`), fixed
and rerun. Means per arm (n=3):

| Arm | Skills loaded | newtypes | enums | stringly ids/KLOC | expect/KLOC | weak asserts | clippy/KLOC | tests | min | USD |
|---|---|---|---|---|---|---|---|---|---|---|
| none | 0 | 0 | 3.0 | 22.3 | 9.2 | 14.3 | 7.4 | 43 | 5.8 | 0.9 |
| skills | 6,6,5 | 2.3 | 3.3 | 24.7 | 5.3 | 3.7 | 3.7 | 71 | 9.5 | 1.7 |
| router | 7,7,7 | 2.7 | 4.3 | 21.9 | 5.1 | 5.0 | 2.7 | 58 | 8.5 | 1.5 |
| hook | 3,3,5 | 3.3 | 3.7 | 17.4 | 2.8 | 0.3 | 2.3 | 60 | 9.2 | 1.6 |
| context | 0 (no plugin) | 9.3 | 8.0 | 11.3 | 3.2 | 6.3 | 1.9 | 39 | 9.8 | 1.6 |

Reading: in greenfield the skills load by themselves on a plain prompt (v1 refactor: never). Every
treatment beats `none` on the detectors; the 68-line checklist in CLAUDE.md moves types the most.
Blind copies in `blind/p1/`, key in `keys/blind-p1.tsv`. P4 list in `keys/p4.list`
(none, skills, context x Haiku 5.5, Opus 5.5 x 2).

### P1 blind dimension scores (0 to 4 each, six dimensions summed; `overall` is separate)

| Arm | types | errors | ownership | simplicity | comments | tests | overall | sum6 (runs) |
|---|---|---|---|---|---|---|---|---|
| context | 4.00 | 3.67 | 2.67 | 1.33 | 3.33 | 3.67 | 3.00 | 18.67 (19, 17, 20) |
| skills | 2.33 | 3.33 | 3.67 | 2.67 | 3.00 | 3.33 | 3.67 | 18.33 (19, 19, 17) |
| router | 1.67 | 3.33 | 3.33 | 2.67 | 4.00 | 3.00 | 2.67 | 18.00 (18, 19, 17) |
| hook | 2.00 | 3.00 | 2.67 | 2.33 | 3.33 | 3.00 | 2.67 | 16.33 (16, 15, 18) |
| none | 0.33 | 1.67 | 3.00 | 2.00 | 2.67 | 1.67 | 1.33 | 11.33 (12, 11, 11) |

No overlap: worst treated run 15, best untreated 12. `context` wins types but is last on
simplicity (single-use types, copied newtype boilerplate). `skills` is best on `overall`.

Baseline for P2/P3: `p1-04` (none, sum 11, tie with p1-07 broken by run number), copied to
`baseline/`. Deviation from the plan: P2 and P3 run every arm (runs are cheap): P2 none, context,
skills, router, hook; P3 none, skills, router, audit, context, hook. List `keys/p23.list`, 33 runs.
P4 batch (Haiku, Opus) still running when P2/P3 was launched.

### P2 and P3 mechanical results (33 runs, Sonnet 5.5, baseline p1-04; all pass hidden tests)

Baseline: newtypes 0, enums 3, expect 12.27/KLOC, weak asserts 20, clippy 5.11/KLOC, clone 3.07.

P2 feature-add (80/80 in all 15):
| Arm | Skills loaded | newtypes | expect/KLOC | weak asserts | min |
|---|---|---|---|---|---|
| none | 0,0,0 | 0,0,0 | 16.0,17.0,15.8 | 21,22,23 | 2.3 |
| skills | 0,0,0 | 0,0,0 | 16.0,16.0,17.4 | 21,22,21 | 2.5 |
| router | 7,7,7 | 0,0,0 | 16.0,15.5,15.4 | 20,21,22 | 2.6 |
| hook | 2,0,1 (fired 3/3) | 0,0,0 | 16.1,17.8,16.0 | 20,20,20 | 3.5 |
| context | none | 2,2,1 | 15.2,15.3,14.6 | 23,27,21 | 3.0 |
Reading: skills as-is do not load on a feature request (0 of 3); new code copies the style around
it in every arm; only the always-in-context checklist adds a few newtypes.

P3 refactor (58/58 in all 18):
| Arm | Skills loaded | newtypes | enums | expect/KLOC | clone/KLOC | weak asserts | commits | min |
|---|---|---|---|---|---|---|---|---|
| none | 0 | 0,0,0 | 4,4,4 | 12.1,12.0,11.9 | 3.0 | 20,20,20 | 3-4 | 1.2 |
| skills | 1,2,1 | 0,0,0 | 3,3,3 | 12.2,12.2,12.3 | 3.1 | 20,20,20 | 2-4 | 0.9 |
| router | 7,7,7 | 0,0,0 | 3,4,3 | 12.1,11.8,12.2 | 3.0 | 20,20,20 | 3-5 | 1.6 |
| audit | 1 (rust-audit) | 0,0,0 | 3,3,3 | 2.0,1.0,2.1 | 3.1 | 15,15,16 | 4-5 | 1.6 |
| hook | 1 (fired 3/3) | 0,0,0 | 3,3,3 | 12.3,1.0,1.0 | 3.1 | 8,15,9 | 2-5 | 1.4 |
| context | none | 0,3,7 | 6,4,6 | 0.9,0.9,2.5 | 0.0,1.9,0.8 | 20,20,15 | 6-12 | 4.7 |
Reading: same as v1 on a small crate. Loading the guides (router) changes nothing. A list of what
to look for (audit), a detector handing findings back (hook), or the checklist in context changes
the code; context changes the most and takes the longest.

Blind sets with BASE in `blind/p2`, `blind/p3`. Reviews: p2 types, errors, tests, overall; p3 all seven.

### P3 blind dimension scores (refactor, BASE included; BASE = 13)

| Arm | types | errors | ownership | simplicity | comments | tests | overall | sum6 (runs) |
|---|---|---|---|---|---|---|---|---|
| audit | 2.00 | 3.33 | 2.67 | 3.00 | 4.00 | 3.00 | 3.00 | 18.00 (19, 17, 18) |
| context | 3.67 | 3.67 | 3.00 | 1.00 | 3.33 | 2.33 | 3.67 | 17.00 (17, 16, 18) |
| hook | 2.00 | 2.67 | 2.00 | 2.00 | 3.00 | 3.67 | 2.67 | 15.33 (15, 15, 16) |
| router | 2.00 | 2.00 | 2.00 | 2.00 | 3.00 | 2.00 | 2.00 | 13.00 (13, 13, 13) |
| skills | 2.00 | 2.00 | 2.00 | 2.00 | 3.00 | 2.00 | 2.00 | 13.00 (13, 13, 13) |
| none | 2.00 | 2.00 | 2.00 | 2.00 | 2.67 | 2.00 | 2.00 | 12.67 (13, 12, 13) |

router, skills and none are indistinguishable from BASE on every dimension. context again last on
simplicity (single-use types). audit is the only arm that improves simplicity.

### P2 blind (feature-add, new code only; BASE = types 2, errors 3, tests 3), overall pending

| Arm | types | errors | tests | sum3 (runs) |
|---|---|---|---|---|
| context | 3.33 | 3.33 | 2.67 | 9.33 (11, 9, 8) |
| router | 2.00 | 3.67 | 3.00 | 8.67 (9, 7, 10) |
| hook | 3.00 | 2.67 | 3.00 | 8.67 (9, 7, 10) |
| skills | 2.00 | 2.67 | 3.33 | 8.00 (9, 9, 6) |
| none | 1.67 | 3.00 | 2.67 | 7.33 (6, 6, 10) |
Ranges overlap in every pair: no arm is separated from `none` on a feature request.

### P4 mechanical (greenfield, other models, n=2; all 58/58)

| Model | Arm | Skills loaded | newtypes | stringly ids/KLOC | weak asserts | min | USD |
|---|---|---|---|---|---|---|---|
| Haiku 5.5 | none | 0 | 0, 0 | 29.1, 21.9 | 28, 18 | 9.4 | 0.49 |
| Haiku 5.5 | skills | 0, 5 | 0, 6 | 22.4, 14.4 | 5, 18 | 14.2 | 1.19 |
| Haiku 5.5 | context | - | 10, 12 | 13.2, 7.0 | 3, 1 | 16.3 | 1.09 |
| Opus 5.5 | none | 0 | 1, 1 | 35.5, 32.3 | 19, 7 | 8.8 | 2.14 |
| Opus 5.5 | skills | 4, 4 | 5, 5 | 9.4, 10.4 | 0, 0 | 11.8 | 3.13 |
| Opus 5.5 | context | - | 13, 10 | 8.1, 11.2 | 0, 0 | 15.8 | 4.06 |
Haiku loaded the skills in one of two runs; Opus in both, and gets more out of them than Sonnet.
Blind set `blind/p4` (reviews: types, errors, tests, overall).

### P2 blind, complete (overall added)

context 3.33/3.33/2.67/3.33, router 2.00/3.67/3.00/3.67, hook 3.00/2.67/3.00/3.00,
skills 2.00/2.67/3.33/3.33, none 1.67/3.00/2.67/3.00 (types/errors/tests/overall). No separation.

### P6 (iteration on what P1 to P3 showed), launched

Why: (1) the same 68 lines act when they sit in CLAUDE.md and the full guides do not act when
loaded as skills, so is it the channel or the length? (2) `context` over-applies types (last on
simplicity in P1 and P3). (3) `rust-keep-it-simple` ends with "Apply these rules to the code the
change touches. Do not rewrite working code elsewhere", which tells a refactor to do nothing.
Arms (in `arms/`):
- `short`: one skill `rust-rules` whose body is exactly `context/CLAUDE.md` (channel test).
- `limits`: one skill `rust-rules`: checklist + each guide's "Do not over-..." bullets + a scope
  line (add or change: only the code you touch; clean up or review: that code). 121 lines.
- `context2`: the same 121 lines in CLAUDE.md.
Tasks: greenfield (p6g-*), refactor (p6r-*), feature (p6f-*), n=3 each, 27 runs, `keys/p6.list`.
Review plan: blind sets that mix the new runs with anchor runs from P1/P2/P3 (none, context, and
skills or audit) so scores are calibrated in-set and the anchors give a test-retest check.

### P4 blind (greenfield, Haiku and Opus, n=2; five dimensions, max 20)

| Arm/model | types | errors | simplicity | tests | overall | sum4 excl. overall (runs) |
|---|---|---|---|---|---|---|
| context/opus | 4.0 | 4.0 | 2.5 | 3.5 | 3.0 | 14.0 (14, 14) |
| skills/opus | 3.0 | 3.0 | 3.5 | 3.5 | 3.5 | 13.0 (14, 12) |
| context/haiku | 3.5 | 3.0 | 1.5 | 3.5 | 2.5 | 11.5 (11, 12) |
| none/opus | 1.0 | 3.0 | 3.0 | 3.0 | 2.5 | 10.0 (10, 10) |
| skills/haiku | 1.5 | 2.5 | 2.5 | 3.0 | 3.0 | 9.5 (10, 9) |
| none/haiku | 0.5 | 2.0 | 2.5 | 2.5 | 1.5 | 7.5 (7, 8) |
Same ordering as Sonnet in both models: none < skills < context on the sum, context worst on
simplicity, skills best or tied on overall. Haiku with the checklist beats Opus without it.

### P6 mechanical (Sonnet 5.5, n=3; all pass hidden tests)

Greenfield (compare P1: none 0 newtypes, skills 2.3, context 9.3):
| Arm | skill loaded | newtypes | enums | expect/KLOC | min |
|---|---|---|---|---|---|
| short | 3/3, first call | 7, 8, 9 | 10, 8, 6 | 3.6, 0.8, 3.8 | 7.4 |
| limits | 3/3, first call | 2, 3, 3 | 7, 4, 4 | 5.0, 3.5, 4.4 | 5.8 |
| context2 | - | 4, 3, 6 | 6, 7, 5 | 6.5, 6.2, 7.6 | 9.3 |
Refactor (baseline expect 12.3, newtypes 0; P3 context: newtypes 0,3,7, expect about 1.4):
| short | 3/3 | 3, 0, 3 | 3, 4, 4 | 12.9, 11.9, 1.9 | 1.6 |
| limits | 3/3 | 3, 0, 0 | 4, 4, 3 | 12.7, 11.8, 13.3 | 1.5 |
| context2 | - | 3, 4, 3 | 5, 4, 4 | 14.4, 13.4, 14.4 | 3.6 |
Feature (baseline newtypes 0): newtypes 0 in 8 of 9 runs, expect unchanged in all. No arm moves it.
Reading so far: in greenfield one short skill equals CLAUDE.md (channel does not matter there);
in refactor the skill channel is weaker than CLAUDE.md; the "But:" limits cut the type count and
stop the `expect` removal (one limit says an unwrap next to its check is fine).
Blind sets with anchors: `blind/p6g` (9 new + P1 none, skills, context), `blind/p6r` (9 new + P3
none, audit, context + BASE). Feature runs not blind-reviewed (no mechanical movement).

### P6 blind (anchors from P1/P3 re-scored in the same set)

Greenfield, five dimensions (max 20):
| Arm | types | errors | simplicity | tests | overall | sum (runs) |
|---|---|---|---|---|---|---|
| context (P1 anchor) | 3.67 | 3.67 | 2.00 | 3.33 | 3.00 | 12.67 (11, 12, 15) |
| short | 3.33 | 3.67 | 2.67 | 3.00 | 3.67 | 12.67 (13, 12, 13) |
| context2 | 2.67 | 3.00 | 2.67 | 3.67 | 2.67 | 12.00 (12, 12, 12) |
| limits | 2.00 | 3.67 | 3.33 | 2.67 | 3.00 | 11.67 (12, 12, 11) |
| skills (P1 anchor) | 1.33 | 3.00 | 2.67 | 3.67 | 2.00 | 10.67 (12, 10, 10) |
| none (P1 anchor) | 0.33 | 1.33 | 2.00 | 2.67 | 1.33 | 6.33 (6, 5, 8) |
Refactor, four dimensions (max 16), BASE = 4:
| context (P3 anchor) | 3.67 | 3.67 | 1.33 | - | 2.67 | 8.67 (8, 9, 9) |
| context2 | 3.33 | 3.33 | 1.33 | - | 3.33 | 8.00 (8, 8, 8) |
| audit (P3 anchor) | 1.00 | 2.67 | 3.00 | - | 3.00 | 6.67 (7, 6, 7) |
| short | 2.33 | 2.00 | 2.00 | - | 3.00 | 6.33 (6, 5, 8) |
| limits | 2.00 | 1.67 | 2.00 | - | 3.33 | 5.67 (7, 5, 5) |
| none (P3 anchor) | 1.00 | 1.00 | 2.00 | - | 2.67 | 4.00 (4, 4, 4) |
Anchor test-retest: p1 vs p6g 36 scores, 21 identical, 33 within one; p3 vs p6r 40 scores, 20
identical, 40 within one.

v2 totals: 87 runs, USD 82.53 API-equivalent, 467 agent minutes. Final report being written to
`REPORT.md`.

## 2026-11-23

### Consumption of the 87 earlier runs (`bin/usage.py`)

Cost and agent time relative to `none`, Sonnet 5.5, n=3: greenfield skills 1.83x/1.66x, context
1.69x/1.71x, short 1.27x/1.28x, hook 1.70x/1.61x; feature skills 1.07x, context 1.30x, short 1.06x,
hook 1.45x; refactor skills 0.97x, context 3.40x, short 1.31x, hook 1.28x. The extra is work done
(greenfield context writes 90k output tokens against 53k), not rule text in context.

### P7 and P8: hook delivery A/B (54 runs, Sonnet 5.5, n=3, all hook arms carry the 68-line checklist and no skills)

Arms: `hk-start` (SessionStart prints the checklist), `hk-start-stop` (same plus a Stop detector
limited to the Rust lines the session added), `hk-pre2` (PreToolUse adds the checklist as context
before the first Rust write), `hk-gate2` (PreToolUse rejects the first Rust write with the checklist
as the reason). `hk-pre` and `hk-gate` (P7) matched only Edit/Write/MultiEdit and fired in 4 of 18
runs, because the agent writes most Rust through Bash heredocs; they count as `none`. The `2` arms
(P8) also match Bash commands that write `.rs` files or run `cargo init`/`cargo new` and fired in 18
of 18. Acceptance: 53 of 54 pass everything; p7g-36 (a P7 gate run whose hook did not fire) fails 2
of 58.

Blind scores (anchors `none`, `context`, `short` from P1/P2/P3/P6 re-scored in the same sets; anchor
test-retest against the earlier reviews: 123 repeated scores, 122 within one point, 92 identical).
Cost is relative to `none` on the same task.

| Arm | Greenfield sum of 16 (runs) | cost | Refactor sum of 12, BASE 5 (runs) | cost | Feature sum of 12 (runs) | cost |
|---|---|---|---|---|---|---|
| hk-start-stop | 13.00 (12, 14, 13) | 1.68x | 9.00 (10, 9, 8) | 2.70x | 9.00 (10, 9, 8) | 1.39x |
| context | 12.67 (12, 13, 13) | 1.69x | 8.67 (8, 9, 9) | 3.40x | 9.67 (11, 9, 9) | 1.30x |
| short | 12.67 (13, 13, 12) | 1.27x | 7.33 (6, 7, 9) | 1.31x | 9.33 (8, 11, 9) | 1.06x |
| hk-gate2 | 11.67 (12, 12, 11) | 1.78x | 7.67 (8, 8, 7) | 1.50x | 8.67 (8, 9, 9) | 1.18x |
| hk-start | 11.33 (12, 9, 13) | 1.59x | 7.33 (7, 7, 8) | 1.56x | 9.33 (10, 8, 10) | 1.12x |
| hk-pre2 | 9.67 (7, 10, 12) | 1.31x | 7.67 (7, 9, 7) | 1.54x | 8.67 (9, 7, 10) | 1.06x |
| none | 6.33 (6, 5, 8) | 1.00x | 5.00 (5, 5, 5) | 1.00x | 7.00 (6, 5, 10) | 1.00x |

Dimensions: greenfield types, errors, simplicity, tests; refactor types, errors, simplicity; feature
types, errors, tests (new code against BASE). Every hook arm separates from `none` on greenfield and
refactor with no overlap. On feature the rule arms average 1.7 to 2.7 points above `none` but one
`none` run scored 10, so the ranges overlap; the mechanical `expect` density does not move.

### v3: description probes (72 short runs, `v3/descriptions/probe.md`, `guidance.md`, `evidence.md`)

Loads in 3 runs per cell (greenfield, feature, refactor): six narrow skills with today's descriptions
0, 0, 0; opening with task verbs 0, 0, 0; task list appended 0, 0, 0; imperative 0, 0, 1; rewritten
after the published guidance 0, 0, 0; three task-level skills 3, 3, 3 (always the matching one, never
a wrong one); the same plus the six narrow skills 3, 3, 3 (narrow ones never loaded); one entry skill
3, 3, 3. The control did not repeat the P1 greenfield self-loading (4 of 6 runs on 11-20, 0 of 3 on
11-23, same client version and opening moves). Cost USD 25.29.

## 2026-11-23: P9, agent-written ast-grep queries (Niko's suggestion)

Arm `astgrep`: the `audit` arm with step 1 of `rust-audit` changed to "for each checklist item,
write ast-grep queries that find where it likely applies, run them, check each hit against the
rule", plus a short ast-grep primer (one pattern, one verified multi-line YAML rule). Everything
else is byte-identical to `audit`. ast-grep 0.51.3 from Homebrew. Refactor prompt, baseline
p1-04, Sonnet 5.5, three runs. Mechanical only, no blind review.

| Run | Arm | Hidden tests | Turns | Min | USD | expect /KLOC | Stringly ids /KLOC | Lint set /KLOC | Weak asserts | Newtypes | Commits |
|---|---|---|---|---|---|---|---|---|---|---|---|
| BASE | - | 58/58 | - | - | - | 12.27 | 19.43 | 5.11 | 20 | 0 | - |
| p3-12 | audit | 58/58 | 15 | 1.8 | 0.34 | 2.02 | 20.24 | 3.04 | 15 | 0 | 5 |
| p3-14 | audit | 58/58 | 13 | 1.4 | 0.29 | 1.02 | 21.41 | 5.10 | 15 | 0 | 4 |
| p3-16 | audit | 58/58 | 14 | 1.5 | 0.31 | 2.05 | 20.51 | 5.13 | 16 | 0 | 5 |
| p9r-01 | astgrep | 58/58 | 15 | 1.5 | 0.32 | 1.02 | 21.34 | 3.05 | 15 | 0 | 5 |
| p9r-02 | astgrep | 58/58 | 21 | 2.0 | 0.42 | 1.02 | 21.45 | 3.06 | 15 | 0 | 5 |
| p9r-03 | astgrep | 58/58 | 15 | 1.6 | 0.32 | 2.03 | 21.28 | 5.07 | 15 | 0 | 4 |

- All three ran ast-grep once, as one loop of 9 to 11 single-node patterns (`$X.expect($$$)`,
  `$X.unwrap()`, `$X.clone()`, `$X as $T`, `let _ = $X`, `$X.ok()`, `$X.unwrap_or_default()`,
  `#[allow($$$)]`) plus `assert!($X.is_ok())`-style test patterns. None wrote a YAML rule, none
  reused the primer's multi-line rule, none queried the type, simplicity or comment items.
- Same fixes as `audit`: required-parameter getters instead of `expect("required")`, exact test
  assertions, named limits instead of casts, lookups done once. Same cost.
- Reading: on this crate the search tool makes no difference; what the agent writes is the same
  set the `rg` lines already give. Untested: a fixed, verified ast-grep rule set per checklist
  item, the same in the Stop hook, and the larger SQS crate where regex noise is higher.

## 2026-11-23: P10, a fixed ast-grep rule set

Arm `astgrep-fixed`: the `audit` arm with step 1 of `rust-audit` changed to "run the ast-grep rules
that ship with this skill" (`ast-grep scan -c <skill>/rules/sgconfig.yml --report-style short src
tests`), and steps 1-2 worded as in `astgrep`. 51 rules in `skills/rust-audit/rules/rules/`, one
file per guide, each message naming the guide and its checklist item; 51 rule tests
(`ast-grep test --skip-snapshot-tests`, all pass, a deliberately wrong case was checked to fail).
On the baseline crate they give 123 hits from 16 rules. Rules were written from the checklists and
`rg` lines, then narrowed against the baseline to cut false positives (`SPEC` as a capitalized
word, `stdin().lock()`, loops over array literals, `#[test]` names, `use serde_json::Value`), so
they are not independent of this crate.

Refactor prompt, baseline p1-04, Sonnet 5.5, three runs. Mechanical only, no blind review.

| Run | Arm | Turns | Min | USD | Enums | Str compares /KLOC | expect /KLOC | Stringly ids /KLOC | Weak asserts | Newtypes | Commits |
|---|---|---|---|---|---|---|---|---|---|---|---|
| BASE | - | - | - | - | 3 | 2.04 | 12.27 | 19.43 | 20 | 0 | - |
| p3-12/14/16 | audit | 13-15 | 1.4-1.8 | 0.29-0.34 | 3, 3, 3 | 2.02-2.05 | 1.02-2.05 | 20.2-21.4 | 15, 15, 16 | 0 | 4-5 |
| p9r-01/02/03 | astgrep | 15-21 | 1.5-2.0 | 0.32-0.42 | 3, 3, 3 | 2.03-2.04 | 1.02-2.03 | 21.3-21.5 | 15, 15, 15 | 0 | 4-5 |
| p10r-01 | astgrep-fixed | 19 | 3.2 | 0.40 | 4 | 0.98 | 0.98 | 21.53 | 7 | 0 | 6 |
| p10r-02 | astgrep-fixed | 18 | 2.5 | 0.35 | 4 | 0.98 | 0.98 | 21.59 | 20 | 0 | 5 |
| p10r-03 | astgrep-fixed | 14 | 2.0 | 0.31 | 4 | 0.98 | 0.98 | 21.53 | 20 | 0 | 4 |

- All three parsed the command name into a `Command` enum at the request boundary, from the
  `dt-string-match-arm` hits (11 string-literal match arms in `broker.rs`). None of the six
  `audit` and `astgrep` runs did; before this only `context` (rules in `CLAUDE.md`, about 3x the
  cost) changed types on the refactor.
- All three replaced `expect("required")` with typed getters (expect 0.98/KLOC in each).
- Weak assertions: one run fixed them (20 -> 7), two left all 20.
- Every run piped the scan into `head -80`, so each saw 80 of the 123 hits, a different subset per
  run (ast-grep output order is not stable). The type rules from `broker.rs` were always inside.
- The `audit` runs never ran the guides' `rg` searches: they read the code and worked from the
  "What to look for" list (p3-12 ran no search; p3-14 one `grep` for test and error patterns).
  So `audit` against `astgrep-fixed` compares "a list of what to look for" with "one command that
  returns labeled hits", not regex with ast-grep.
- Stringly ids and newtypes did not move in any arm. Cost about the same as `audit`.

## 2026-11-24: why the checklist as a skill costs less than the same text in `CLAUDE.md`

Greenfield, Sonnet 5.5, three runs each. `arms/short/skills/rust-rules/SKILL.md` and
`arms/context/CLAUDE.md` are the same 68 lines (the skill adds only its frontmatter), so the
difference is not the text.

| Arm | Delivery | USD | Turns | Output tokens | Lines written (prod + test) | Structs | Tests |
|---|---|---|---|---|---|---|---|
| none | - | 0.89-0.99 | 17-21 | 51k-55k | 1033-1200 + 908-1085 | 11-14 | 41-45 |
| short | skill, loaded at turn 1 | 1.19-1.21 | 20-25 | 65k-69k | 1344-1446 + 800-917 | 24-28 | 37-51 |
| context | `CLAUDE.md` | 1.45-1.80 | 22-31 | 79k-103k | 1448-2352 + 297-1422 | 32-44 | 16-62 |
| hk-start | SessionStart hook | 1.33-1.59 | 17-21 | 75k-96k | 1334-1570 + 1037-1450 | 29-32 | 40-73 |
| hk-start-stop | SessionStart + Stop | 1.50-1.73 | 20-28 | 83k-94k | 1395-1495 + 924-1218 | 31-38 | 41-47 |

- The skill runs wrote about 2,250 lines, the always-present deliveries 2,500 to 2,700: more
  structs and, in the hook arms, more tests. Output tokens follow (67k against 88k to 90k), and
  cache reads follow the turns. Blind greenfield score is the same (`short` 12.67, `context` 12.67).
- `hk-start` (run on 11-23) lands with `context` (11-19), not with `short` (11-20), so the day is
  not the cause.
- Not established: why a skill body leads to less code than the same text as session context.
  Three runs per arm, and `short` was never run next to `context` on the same day.

## 2026-11-24: P11, feature-add with a scope note and a firmer detector

What the earlier feature runs did with the detector (`hk-start-stop`, p7f-01/17/29): it reported 31
to 41 findings in the added lines, the agent fixed 1 or 2 and kept the rest with "the existing
handlers use the same pattern". So on a feature the rules were in context and the findings were
shown; the agent chose the convention of the surrounding code.

Two arms, both `hk-start-stop` with changes:

- `hk-feat-start`: SessionStart prints `hooks/scope.md` before the checklist (new lines follow the
  rules even where the code next to them does not; change existing code just enough when it forces
  new code to break a rule; no rewrite or migration beyond that; crate-wide rules apply only to
  data the change introduces). No Stop hook.
- `hk-feat`: the same, plus the Stop detector with two changes: findings for `unwrap`, `expect`
  and by-value arguments under `tests/` are dropped, and the closing line says "That the existing
  code does the same is not a reason to keep one".

Feature prompt, baseline p1-04, Sonnet 5.5, three runs each. Mechanical only, no blind review.
"Added" and "removed" are lines of `git diff start..HEAD` outside `tests/`.

| Run | Arm | Hidden tests | Turns | Min | USD | `expect` in added lines | Existing lines removed | New structs | Newtypes | Weak asserts (start 20) |
|---|---|---|---|---|---|---|---|---|---|---|
| p2-09/12/13 | none | 80/80 | 12-17 | 1.9-2.9 | 0.42-0.61 | 7, 8, 9 | 5, 1, 97 | 0, 0, 2 | 0 | 21-23 |
| p6f-01/07/08 | short | 80/80 | 16-19 | 2.2-2.8 | 0.47-0.55 | 6, 6, 9 | 3, 6, 8 | 0, 3, 2 | 0-2 | - |
| p2-02/08/10 | context | 80/80 | 18-27 | 3.0-3.1 | 0.63-0.65 | 9, 7, 6 | 13, 5, 2 | 4, 3, 3 | 1-2 | - |
| p7f-04/32/33 | hk-start | 80/80 | 13-15 | 2.5-2.8 | 0.53-0.58 | 12, 8, 11 | 94, 2, 8 | 1, 1, 1 | - | - |
| p7f-01/17/29 | hk-start-stop | 80/80 | 21-26 | 3.5-3.6 | 0.66-0.70 | 9, 12, 9 | 13, 9, 6 | 1, 2, 1 | 0 | 20 |
| p11f-02/04/05 | hk-feat-start | 80/80 | 15-19 | 3.2-3.8 | 0.55-0.67 | 7, 8, 11 | 6, 7, 75 | 4, 3, 4 | 2 | 20-27 |
| p11f-01/03/06 | hk-feat | 80/80 | 20-24 | 4.5 | 0.68-0.72 | 1, 0, 0 | 14, 2, 3 | 4, 2, 5 | 1-2 | 20 |

- `hk-feat` is the first of 21 feature runs with no new `expect`. All three wrote the feature with
  `expect` first, got 19 to 25 findings at stop, and fixed them by adding `required_string`,
  `required_array` and `required_members` getters to the existing `Params` (2 to 14 existing lines
  removed), which is the small enabling change the scope note allows. They also fixed weak
  assertions in the new tests and one added an `EntryId` newtype. They kept queue names and
  handles as `&str` ("the rules say not to retype existing data").
- `hk-feat-start` (the scope note without the detector) is where every earlier arm is: 7 to 11 new
  `expect`. The text at session start does not change the first draft; the detector does.
- Cost is that of `hk-start-stop` (USD 0.68 to 0.72 against 0.66 to 0.70), about 1.4x `none`.
- Not separated: the firmer closing line from the dropped test findings (both changed at once).
  Not reviewed blind. One crate, three runs.

### P11 blind (set `blind/p11f`: 6 new runs, anchors `none`, `context`, `hk-start-stop`, BASE)

Three Opus reviewers, one per dimension, scoring the code each attempt added or changed against
BASE. Sum of 12; BASE = types 2, errors 3, tests 3.

| Arm | Types | Errors | Tests | Sum (runs) | Same runs in the P7 set |
|---|---|---|---|---|---|
| context | 3.33 | 3.67 | 3.67 | 10.67 (12, 10, 10) | 11, 9, 9 |
| hk-feat | 3.33 | 3.00 | 4.00 | 10.33 (11, 10, 10) | - |
| hk-feat-start | 3.33 | 3.00 | 3.33 | 9.67 (9, 9, 11) | - |
| hk-start-stop | 1.67 | 2.67 | 4.00 | 8.33 (9, 9, 7) | 10, 9, 8 |
| none | 1.33 | 2.67 | 2.67 | 6.67 (6, 6, 8) | 6, 5, 10 |

- `hk-feat` and `context` separate from `none` with no overlap (10 to 12 against 6 to 8). In the P7
  set the `none` run p2-13 scored 10 and the ranges overlapped; here it scores 8.
- The scope note is what moves types: both arms that carry it score 3.33 against 1.67 for
  `hk-start-stop`, which has the same checklist and no note. The new tag data is parsed into
  `TagKey`/`TagValue` types in those runs.
- The detector is what moves tests (4.00 with it, 3.33 and 2.67 without).
- The errors reviewer does not see the `expect` removal: `hk-feat` (0 to 1 new `expect`) and
  `hk-feat-start` (7 to 11) both score 3, 3, 3, as does BASE. It accepts a named `expect` after
  `require` and marks most attempts down for `body.len() as u64` instead. So the mechanical gain of
  the detector on errors is not confirmed by the blind review.
- Anchor drift against the P7 set: 27 repeated scores, all within one point; sums move by up to 2.
- Three runs per arm, one reviewer per dimension.
