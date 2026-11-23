I tested the new skills ([`ai-patterns-skills`](https://github.com/fnzko/just-right-rust/tree/30ce8a2/skills)) by letting agents work alone, headless, with and without them, and scoring the code blind.

**Result:** the rules work, the delivery does not. With the rules in front of it the agent writes clearly better Rust (average blind score 1.6 of 4 without rules, 3.2 with them, writing from scratch), but with today's descriptions it almost never loads the skills. One short entry skill, or a hook, fixes that.

**How to read the scores:** reviewers that do not know which run got which rules score each crate from 0 to 4 on types, error handling, simplicity and tests (ownership and comments too in some steps). Unless said otherwise, a number below is the average of those, so 4 is the best. Scores compare only inside one step.

**What we tried, in order**

1. **Refactor of the aws-emulator SQS crate, skills installed, plain "clean this up" request.** No skill loaded, and the code was no better. Scored against the original crate (0 is no better than the original, 22 is the top): 8, 5, 4, 3, 2 with skills, 4, 4, 6, 9 without. ([scores](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/TRIAL-LOG.md?plain=1#L4457))
2. **Why they do not load.** Rewording the descriptions: nothing. Asking for the skills in the prompt: 2 of 8 load. One entry skill that points to the others: all load, every time. ([finding](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/TRIAL-LOG.md?plain=1#L4858))
3. **All skills loaded on the refactor.** No change. Three lines saying what to look for did the work: 7 and 9 with them, 4 and 2 without (same scale as step 1). ([finding](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/TRIAL-LOG.md?plain=1#L5055))
4. **New crate from a spec, hidden acceptance tests, five ways of delivering the rules.** All of them help: 1.9 of 4 with nothing, 2.7 to 3.1 with any of them. ([P1](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L683))
5. **Add a feature to an existing crate.** Nothing moved. The new code copies the style around it. ([P2](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L876))
6. **Refactor of that small crate.** The full guides give back the untouched crate (2.2 of 4, the same as before the refactor). A checklist gives 2.8, an audit skill 3.0. ([P3](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L1050))
7. **Same thing on Haiku and Opus.** Same order on both. Haiku with the checklist beats Opus without it. ([P4](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L1290))
8. **The audit skill on the SQS crate.** 7 and 9 on the scale of step 1, in line with its best runs, in 10 to 12 minutes. ([P5](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L1455))
9. **Short against long.** The six review checklists as one 68-line skill score the same as or above the full guides. ([P6](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L1579))
10. **Eight ways of writing the descriptions, 72 short runs.** Six narrow skills, any wording, including one that follows the published guidance: 1 load in 45. One entry skill, or three task skills (new code, refactor, add a feature): 27 of 27, always the right one. ([probes](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L2706))
11. **Rules delivered by a hook, 54 runs.** The checklist at session start plus a detector at stop matches the checklist in `CLAUDE.md`. A hook bound only to Edit and Write fired in 4 of 18 runs, because the agent writes Rust through shell heredocs. ([hooks](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L2840))
12. **Tokens and time.** From scratch the rules cost 27% to 69% more. That is work done (more types, more tests), not rule text in context. ([consumption](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L2778))

**Where it ended** (average blind score of 4, Sonnet, 3 runs per cell; cost is tokens from scratch, relative to no rules)

| Delivery | From scratch | Refactor | Add a feature | Cost |
|---|---|---|---|---|
| Hook: start + detector | 3.3 | 3.0 | 3.0 | +68% |
| Checklist in `CLAUDE.md` | 3.2 | 2.9 | 3.2 | +69% |
| Short checklist skill | 3.2 | 2.4 | 3.1 | +27% |
| Nothing | 1.6 | 1.7 | 2.3 | 0 |

140 of 141 runs on the small crate pass the hidden tests, so no gain cost behavior.

**Still open:** adding a feature to code written in another style. In the last round the rules add about 0.7 of 4 there, and the runs with and without rules overlap.

**Limits:** two or three runs per cell, one kind of service, small crates, model reviewers only (122 of 123 repeated scores within one point), and I wrote the variant texts.

Everything is in [this repo](https://github.com/fnzko/jrr-skills-trial): [full report](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md), [first trial log](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/TRIAL-LOG.md).
