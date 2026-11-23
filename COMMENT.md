I tested the new skills ([`ai-patterns-skills`](https://github.com/fnzko/just-right-rust/tree/30ce8a2/skills)) by letting agents work alone, headless, with and without them: 16 refactors of the aws-emulator SQS crate, then 141 runs on a small queue service with hidden acceptance tests, all reviewed blind.

Short version: the rules work, the delivery does not. With the rules in front of it the agent writes clearly better Rust, but with today's descriptions it almost never loads the skills. One short entry skill, or a hook, fixes that.

Everything is in [this repo](https://github.com/fnzko/jrr-skills-trial): [full report](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md), [log of the first trial](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/TRIAL-LOG.md).

**What I found**

1. **The rules help.** From scratch, blind scores go from 6 to 13 of 16, on Haiku, Sonnet and Opus. Haiku with the checklist beats Opus without it. ([from scratch](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L683), [other models](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L1290))
2. **The skills do not get loaded.** The agent picks skills once, at the start, by matching the request, and the six descriptions name things that happen inside the work ("when defining structs", "reaches for `unwrap`"). On "clean this crate up" only `rust-keep-it-simple` loads, on "add this feature" none. ([first trial](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/TRIAL-LOG.md?plain=1#L4797), [116 transcripts](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v3/descriptions/evidence.md), [the descriptions side by side](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L2622))
3. **Rewording the descriptions is not enough.** Five wordings, one of them following the published guidance: 1 load in 45 short runs. One entry skill, or three task-level skills (new code, refactor, add a feature), loaded in 27 of 27, always the right one. ([probes](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L2706), [variant texts](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v3/descriptions/probe.md), [guidance and sources](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v3/descriptions/guidance.md))
4. **Short beats long.** The six review checklists as one 68-line skill score the same as or above the full guides. ([results](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L1579))
5. **On existing code the agent has to be told what to look for.** The guides, once loaded, give back the untouched crate. A checklist, an audit skill or a detector change it. ([refactor](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L1050), [first trial](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/TRIAL-LOG.md?plain=1#L5055))
6. **A hook does it without depending on the agent's choice.** The checklist at session start plus a detector at stop, limited to the lines the session added, matches the checklist in `CLAUDE.md`. ([hook A/B](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L2840), [the hooks](https://github.com/fnzko/jrr-skills-trial/tree/c3f5de5/v2/arms/hk-start-stop/hooks))
7. **Adding a feature is still weak.** New code copies the style around it. The rules add about 2 points of 12 and the ranges overlap. ([feature runs](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L876), [why I think so](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L2598))

**By delivery** (blind score, Sonnet, 3 runs per cell; cost is from scratch, relative to no rules)

| Delivery | From scratch (of 16) | Refactor (of 12) | Feature (of 12) | Cost |
|---|---|---|---|---|
| Hook: start + detector | 13.0 | 9.0 | 9.0 | +68% |
| Checklist in `CLAUDE.md` | 12.7 | 8.7 | 9.7 | +69% |
| Short checklist skill | 12.7 | 7.3 | 9.3 | +27% |
| Nothing | 6.3 | 5.0 | 7.0 | 0 |

The extra cost is work done (more types, more tests), not rule text in context ([consumption](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L2778)). 140 of 141 runs pass the hidden tests.

**What I would change**

- Add a short entry skill with the checklists ([the one tested](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/arms/short/skills/rust-rules/SKILL.md)), or three by task ([the ones probed](https://github.com/fnzko/jrr-skills-trial/tree/c3f5de5/v3/variants/v4/skills)), and keep the guides as reference.
- Make the plugin's hooks do something: the checklist at `SessionStart` and a detector at `Stop`. A hook bound only to Edit and Write fired in 4 of 18 runs, because the agent writes most Rust through shell heredocs, so it has to match Bash too.
- Reword [this line](https://github.com/fnzko/just-right-rust/blob/30ce8a2/skills/rust-keep-it-simple/SKILL.md#L191) of `rust-keep-it-simple`: "Do not rewrite working code elsewhere to match them" tells an agent doing a refactor to do nothing. I did not test that change alone.
- For the eval harness: it grades pass or fail on hidden tests, and these skills change how the code reads, not whether it works, so it needs a quality measure next to that. ([notes](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L2570))

**Limits**

Two or three runs per cell, one kind of service, small crates, reviewers and authors from the same model family, no human review, and the variant texts were written for this trial. 122 of 123 repeated blind scores differ by one point or less ([reliability](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L2840)).

Commits of the last round: [hook variants](https://github.com/fnzko/jrr-skills-trial/commit/0910346), [hook runs](https://github.com/fnzko/jrr-skills-trial/commit/1f06bc6), [rerun with the fixed matcher](https://github.com/fnzko/jrr-skills-trial/commit/79ca64a), [blind reviews](https://github.com/fnzko/jrr-skills-trial/commit/7920cc2), [description probes](https://github.com/fnzko/jrr-skills-trial/commit/65a422d). What I got wrong on the way and corrected: [second round](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L179), [third round](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L3112).
