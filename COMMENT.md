I tested the new skills ([`ai-patterns-skills`](https://github.com/fnzko/just-right-rust/tree/30ce8a2/skills)) by letting agents work alone, headless, with and without them, and scoring the code blind.

**Result:** the rules work, the delivery does not. With the rules in front of it the agent writes clearly better Rust (6 to 13 of 16 from scratch), but with today's descriptions it almost never loads the skills. One short entry skill, or a hook, fixes that.

**What we tried, in order**

1. **Refactor of the aws-emulator SQS crate, skills installed, plain "clean this up" request.** No skill loaded, and the code was no better: 8, 5, 4, 3, 2 with skills against 4, 4, 6, 9 without. ([scores](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/TRIAL-LOG.md?plain=1#L4457))
2. **Why they do not load.** Rewording the descriptions: nothing. Asking for the skills in the prompt: 2 of 8 load. One entry skill that points to the others: all load, every time. ([finding](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/TRIAL-LOG.md?plain=1#L4858))
3. **All skills loaded on the refactor.** No change. Three lines saying what to look for did the work: 7 and 9 with them, 4 and 2 without. ([finding](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/TRIAL-LOG.md?plain=1#L5055))
4. **New crate from a spec, hidden acceptance tests, five ways of delivering the rules.** All of them help: 11.3 of 24 with nothing, 16.3 to 18.7 with any of them. ([P1](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L683))
5. **Add a feature to an existing crate.** Nothing moved. The new code copies the style around it. ([P2](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L876))
6. **Refactor of that small crate.** The full guides give back the untouched crate (13 of 24). A checklist gives 17, an audit skill 18. ([P3](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L1050))
7. **Same thing on Haiku and Opus.** Same order on both. Haiku with the checklist beats Opus without it. ([P4](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L1290))
8. **The audit skill on the SQS crate.** 7 and 9, in line with the best of step 1, in 10 to 12 minutes. ([P5](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L1455))
9. **Short against long.** The six review checklists as one 68-line skill score the same as or above the full guides. ([P6](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L1579))
10. **Eight ways of writing the descriptions, 72 short runs.** Six narrow skills, any wording, including one that follows the published guidance: 1 load in 45. One entry skill, or three task skills (new code, refactor, add a feature): 27 of 27, always the right one. ([probes](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L2706))
11. **Rules delivered by a hook, 54 runs.** The checklist at session start plus a detector at stop matches the checklist in `CLAUDE.md`. A hook bound only to Edit and Write fired in 4 of 18 runs, because the agent writes Rust through shell heredocs. ([hooks](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L2840))
12. **Tokens and time.** From scratch the rules cost 27% to 69% more. That is work done (more types, more tests), not rule text in context. ([consumption](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L2778))

**Where it ended** (blind score, Sonnet, 3 runs per cell; cost is from scratch)

| Delivery | From scratch (of 16) | Refactor (of 12) | Feature (of 12) | Cost |
|---|---|---|---|---|
| Hook: start + detector | 13.0 | 9.0 | 9.0 | +68% |
| Checklist in `CLAUDE.md` | 12.7 | 8.7 | 9.7 | +69% |
| Short checklist skill | 12.7 | 7.3 | 9.3 | +27% |
| Nothing | 6.3 | 5.0 | 7.0 | 0 |

140 of 141 runs on the small crate pass the hidden tests, so no gain cost behavior.

**What I would change**

- One short entry skill with the checklists ([tested](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/arms/short/skills/rust-rules/SKILL.md)), or three by task. The six guides stay as reference.
- Plugin hooks that do something: checklist at `SessionStart`, detector at `Stop`, matching Bash too ([tested](https://github.com/fnzko/jrr-skills-trial/tree/c3f5de5/v2/arms/hk-start-stop/hooks)).
- Reword [this line](https://github.com/fnzko/just-right-rust/blob/30ce8a2/skills/rust-keep-it-simple/SKILL.md#L191) of `rust-keep-it-simple`; it tells an agent doing a refactor to touch nothing. Not tested alone.
- The eval harness grades pass or fail on hidden tests. These skills change how the code reads, not whether it works, so it needs a quality measure next to that.

**Still open:** adding a feature to code written in another style. In the last round the rules add about 2 points of 12 there, with overlapping ranges.

**Limits:** two or three runs per cell, one kind of service, small crates, model reviewers only (122 of 123 repeated scores within one point), and I wrote the variant texts.

Everything is in [this repo](https://github.com/fnzko/jrr-skills-trial): [full report](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md), [first trial log](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/TRIAL-LOG.md).
