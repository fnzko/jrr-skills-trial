I tested the new skills ([`ai-patterns-skills`](https://github.com/fnzko/just-right-rust/tree/30ce8a2/skills)) by letting agents work alone, headless, with and without them. Other model sessions then reviewed the code without knowing which run had the rules.

**Short version:** the rules work, the delivery does not. When the agent has the rules in front of it, reviewers rate its Rust about twice as high. But with today's descriptions it almost never loads the skills. One short entry skill, or a hook, fixes that.

**What we tried, in order**

1. **Refactor of the aws-emulator SQS crate, skills installed, plain "clean this up" request.** The agent loaded no skill, and the result was no better than without them. ([details](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/TRIAL-LOG.md?plain=1#L4797))
2. **Why they do not load.** The agent picks skills once, at the start, from the request. Rewording the descriptions changed nothing. One entry skill pointing to the others got them all loaded. ([details](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/TRIAL-LOG.md?plain=1#L4858))
3. **All skills loaded on the refactor.** Still no better. What made the difference was three lines telling the agent what to look for. ([details](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/TRIAL-LOG.md?plain=1#L5055))
4. **New crate from a spec, with hidden acceptance tests.** Here the rules clearly help, whichever way they are delivered. ([details](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L683))
5. **Add a feature to an existing crate.** No effect. The new code copies the style around it. ([details](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L876))
6. **Refactor of that small crate.** With the full guides loaded, the agent hands back a crate no better than the original. With a short checklist or an audit skill it does the refactor. ([details](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L1050))
7. **Haiku and Opus.** Same picture on both. Haiku with the checklist does better than Opus without it. ([details](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L1290))
8. **The audit skill on the SQS crate.** As good as the best runs of step 1, in 10 to 12 minutes. ([details](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L1455))
9. **Short against long.** The six review checklists as one 68-line skill do as well as the full guides. ([details](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L1579))
10. **Eight ways of writing the descriptions.** The six narrow skills loaded once in 45 runs, whatever the wording. One entry skill, or three skills by task (new code, refactor, add a feature), loaded in 27 of 27. ([details](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L2706))
11. **Rules delivered by a hook.** The checklist at session start plus a check of the new lines at the end works as well as the checklist in `CLAUDE.md`. A hook bound only to Edit and Write almost never fires (4 of 18 runs), because the agent writes Rust through shell heredocs. ([details](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L2840))
12. **Cost.** From scratch the rules cost 27% to 69% more tokens. That is extra work the agent does (more types, more tests), not the rule text. ([details](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md?plain=1#L2778))

**Still open:** adding a feature to code written in another style. The last round shows a small gain there, not a clear one.

**How much to trust it:** two or three runs per case, one kind of service, small crates, model reviewers only, and I wrote the variant texts. Every run but one still passes the hidden tests.

Numbers, tables and method are in the [full report](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/v2/REPORT.md) and the [log of the first trial](https://github.com/fnzko/jrr-skills-trial/blob/c3f5de5/TRIAL-LOG.md), in [this repo](https://github.com/fnzko/jrr-skills-trial).
