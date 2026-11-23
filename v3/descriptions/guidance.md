# Skill descriptions: published guidance and corrections

Research date: 2026-11-23. Scope: how the `description` field of a `SKILL.md` should be written so a coding agent loads the skill, applied to the six just-right-rust skills.

## Sources

| Id | Source | URL |
| - | - | - |
| S1 | Anthropic, Skill authoring best practices | https://platform.claude.com/docs/en/agents-and-tools/agent-skills/best-practices |
| S2 | Claude Code docs, Extend Claude with skills (frontmatter reference, troubleshooting, content lifecycle) | https://code.claude.com/docs/en/skills |
| S3 | Agent Skills specification | https://agentskills.io/specification |
| S4 | agentskills.io, Optimizing skill descriptions | https://agentskills.io/skill-creation/optimizing-descriptions |
| S5 | `skill-creator` skill, anthropics/skills | https://github.com/anthropics/skills/blob/main/skills/skill-creator/SKILL.md |
| S6 | `skill-creator` description improver prompt | https://github.com/anthropics/skills/blob/main/skills/skill-creator/scripts/improve_description.py |
| S7 | OpenAI, Build skills (Codex; `developers.openai.com/codex/skills` redirects here) | https://learn.chatgpt.com/docs/build-skills |
| S8 | GitHub, Creating agent skills for Copilot | https://docs.github.com/en/copilot/how-tos/use-copilot-agents/coding-agent/create-skills |
| S9 | GitHub, About agent skills | https://docs.github.com/en/copilot/concepts/agents/about-agent-skills |
| S10 | Gemini CLI, Agent Skills | https://geminicli.com/docs/cli/skills/ |
| S11 | Gemini CLI, Creating skills | https://geminicli.com/docs/cli/creating-skills/ |

No Antigravity-specific page was found; Antigravity reads the same format as Gemini CLI, so S10/S11 stand in for it. No blog posts were used.

## Patterns

### P1. The description is the only trigger; the body is read after the decision

Every vendor says the same: only `name` and `description` are in context at startup, and the agent matches the task against the description to decide whether to load the body.

- S4: the description "carries the entire burden of triggering".
- S5: "All \"when to use\" info goes here, not in the body."
- S7 (Codex), S8 (Copilot), S10 (Gemini: the model calls `activate_skill` when a task matches the description), S2 (Claude Code).
- Consequence: trigger text inside the body (for example "use this when reviewing") does nothing for loading.

### P2. Say what it does and when to use it, both

- S1, S3: "Should describe what the Skill does and when to use it." S3 adds: "include specific keywords that help agents identify relevant tasks".
- S1 example shape: one sentence of capabilities, then "Use when ...".
- S8: "what the skill does, and when Copilot should use it".

### P3. Describe the user's request (intent), not the skill's mechanics

- S4: "Focus on user intent, not implementation ... The agent matches against what the user asked for."
- S6: focus on "what they are trying to achieve, vs. the implementation details of how the skill works".
- S2 troubleshooting: "Check the description includes keywords users would naturally say."
- S11: name the tasks the skill covers and the words that should trigger it.
- Tasks versus symptoms: the guidance always frames triggers as requests or situations the user is in ("when the user mentions PDFs", "when exploring a new repo"), not as code-level events inside the work. Code tokens are fine as keywords (S1 uses `.xlsx`), but they match the request, not edits the agent will make later.

### P4. Person and voice: imperative "Use this skill when", never first or second person

- S1: "Always write in third person", because the description is injected into the system prompt; avoid "I can help you" and "You can use this".
- S4 and S6: phrase it as an instruction to the agent, "Use this skill when..." rather than "This skill does...".
- These agree in practice: the S1 and S3 examples use a capability clause followed by "Use when ...". What is ruled out is "I" and "you".

### P5. Be a little pushy, and include requests that do not name the domain

- S5: Claude tends to "undertrigger" skills, so make descriptions "a little bit \"pushy\"", e.g. "Make sure to use this skill whenever ... even if they don't explicitly ask for a 'dashboard.'"
- S4: "Err on the side of being pushy", listing contexts "including cases where the user doesn't name the domain directly".
- Counterweight, S2: if a skill "triggers too often", "Make the description more specific". S4: an over-broad description "triggers when it shouldn't"; fix false triggers by stating what the skill does not do.

### P6. Put the key use case first

- S2: "Put the key use case first", because the listing truncates.
- S7: put the main use case and trigger words first so matching survives shortening; explain "exactly when this skill should and should not trigger".

### P7. Length

- Hard limit 1,024 characters (S1, S3, S4).
- Claude Code caps `description` plus `when_to_use` at 1,536 characters per entry, and drops whole descriptions (least used first) when the listing exceeds about 1% of the context window (S2).
- Codex: the initial skills list is capped at about 2% of context or 8,000 characters and shortens descriptions first (S7).
- Recommended size: "a few sentences to a short paragraph" (S4); "not more than about 100-200 words" (S6); metadata "~100 words" (S5).

### P8. Agents skip skills for tasks they think they can do alone

- S5: "Claude only consults skills for tasks it can't easily handle on its own"; complex, multi-step or specialized queries "reliably trigger skills when the description matches".
- S4 says the same: simple requests may not trigger a matching skill.

### P9. Narrow versus broad, and distinctiveness

- S7: keep each skill focused on one job.
- S6: the description "competes with other skills for Claude's attention" and should be "distinctive and immediately recognizable".
- S1 Pattern 2: one skill with domain reference files loaded on demand (`reference/finance.md`, `reference/sales.md`) as the way to cover several domains without loading all of them.
- No source discusses a set of sibling skills that should all load together for one job, or an entry skill that loads others.

### P10. Test triggering with realistic should and should-not prompts

- S4, S5: about 20 queries, 8 to 10 each, including near-miss negatives and requests where the connection is not obvious; run each 3 times; 60/40 train/validation split; pick by validation score; do not paste keywords from failed queries (overfitting), generalize to the category.
- S2: `claude plugin eval` with a `tool_used: Skill` grader for plugin skills.

### P11. Loaded once, never re-read

- S2: the skill content enters the conversation as one message, Claude Code "does not re-read the skill file on later turns, so write guidance that should apply throughout a task as standing instructions".
- This is about the body after loading. No source says the agent re-checks the skill list mid-task.

## Assessment of the six descriptions

Lengths are in characters, all well under 1,024.

| Skill | Len | What it does (P2) | When to use (P2, P3) | Voice (P4) | Pushy / unnamed domain (P5) | Key use first (P6) | Main problem |
| - | - | - | - | - | - | - | - |
| rust-domain-types | 319 | Clear | Edit-level: "defining structs, enums or function signatures", "parsing", "reviewing code that passes ids ... as `String`" | Opens with imperative rule, then "Use when" | No | No: rule first | Triggers describe edits inside the job, not requests; nothing for "build", "add a feature", "clean up" |
| rust-error-discipline | 435 | Clear but dense (four rules in one sentence) | Edit-level: "handles `Result`", "adds a fallback", "reaches for `unwrap`" | Same | No | No | Same; the token list (`unwrap_or_default`, `let _ =`) matches code the agent writes later, not the prompt |
| rust-ownership | 411 | Clear | Mixed: "fixing a borrow checker error" is a real request; the rest is edit-level | Same | No | No | Only the borrow-error trigger is visible at task start |
| rust-test-quality | 416 | Clear | Partly request-level: "adding a regression test for a fix", "writing tests" | Same | No | No | Closest to working; misses that every feature or spec build includes tests |
| rust-comments | 382 | Clear | Request-level for commits and PRs; edit-level for comments | Same | No | No | Commit/PR triggers are good; comment triggers fire only mid-edit |
| rust-keep-it-simple | 460 | Clear | Contains "adding a feature" and "reviewing a diff", which are requests | Same | No | No | The only one naming a whole job; consistent with it being the one loaded on "clean this crate up" |
| async-rust (contrast) | 143 | Short | Task-level verbs: "Design, implement, debug, or review asynchronous Rust" | Imperative verbs | No | Yes | No "Use when", but its verbs are the jobs a user asks for; covers a domain, not an edit |

Shared pattern: each description is written from inside the edit ("when writing `if`", "when adding `.clone()`"). The agent decides once, at the start (P1, trial F2), by matching the request (P3). A request reads "add pagination", "clean this crate up", "implement SPEC.md", so none of the edit-level triggers can match it. All six also lead with the rule's content instead of the use case (P6) and none is pushy (P5).

## Corrected descriptions

Shape for all six: imperative "Use this skill whenever" + the jobs a user asks for (build from a spec, add a feature, fix a bug, refactor, clean up, review) + the code area as a keyword + "even if the user does not mention X" + one sentence of what it changes. Code tokens are kept at the end as keywords.

### rust-domain-types (446 characters)

```
Use this skill whenever writing, extending, refactoring or reviewing Rust code that defines data: structs, enums, function signatures, config, or request and message parsing. Applies when building a crate from a spec, adding a feature or cleaning up existing code, even if the user never mentions types. Replaces bare `String`, `u64` and `bool` ids, names, states and units with enums and newtypes, and parses external input once at the boundary.
```

### rust-error-discipline (476 characters)

```
Use this skill whenever writing, extending, refactoring or reviewing Rust code that can fail: I/O, parsing, input validation, arithmetic on input, or any `Result` or `Option`. Applies when building a crate from a spec, adding a feature, fixing a bug or cleaning up a crate, even if the user never mentions errors. Removes silent defaults, discarded `Result`s, `unwrap` on recoverable failures and duplicate error reports; keeps panics for bugs and adds context once per layer.
```

### rust-ownership (521 characters)

```
Use this skill whenever writing, extending, refactoring or reviewing Rust code that passes data between functions, loops over collections or shares state behind a lock, and whenever fixing a borrow checker error. Applies when building a crate from a spec, adding a feature, optimizing or cleaning up a crate, even if the user never mentions performance. Removes needless `.clone()`, `.to_string()` and `.collect()`, repeated scans inside loops and long lock scopes by borrowing in signatures and computing once per input.
```

### rust-test-quality (468 characters)

```
Use this skill whenever writing, changing or reviewing Rust tests, including the tests that come with building a crate from a spec, adding a feature, fixing a bug or refactoring, even if the user only asks to make sure it works. Makes `#[test]` functions and `tests/` files fail when the code is wrong: exact assertions instead of `is_ok()`, non-empty or substring checks, expected values not computed by the code under test, and no dependence on timing or test order.
```

### rust-comments (428 characters)

```
Use this skill whenever writing or editing Rust code that carries comments or docs, and whenever writing a commit message or pull request description, including during a spec build, a feature, a bug fix or a cleanup. Keeps `//`, `///`, `SAFETY` and TODO comments to reasons, invariants and contracts; removes comments that restate the code, narrate the edit or record history; keeps each commit to one purpose and PR text short.
```

### rust-keep-it-simple (433 characters)

```
Use this skill whenever writing, extending, refactoring, cleaning up or reviewing Rust code: building a crate from a spec, adding a feature, fixing a bug or simplifying existing code. Adds no trait, generic, helper, field, dependency or compatibility shim the change does not need, changes existing code before adding beside it, deletes what the change makes dead, and writes branches, loops and iterator chains in their direct form.
```

### Optional seventh: an entry skill (not one of the six; backed by trial F3, not by published guidance)

If the goal is "every Rust job loads all six", the guidance on distinctiveness (P9) argues against six near-identical "whenever writing Rust" descriptions. The structure the trial measured as working is one entry skill whose body tells the agent to load the six:

```
Use this skill at the start of any Rust coding task: implementing a crate or spec, adding a feature, fixing a bug, refactoring, cleaning up or reviewing Rust code, even if the user does not ask for quality or style. Loads the just-right-rust rules for domain types, error handling, ownership, tests, comments and simplicity before any code is written.
```

## Conflicts and gaps

1. **"Pushy" and "add contexts" versus trial b1.** S4 and S5 say adding contexts and pushy wording fixes undertriggering. Trial b1 appended "Also use when refactoring or cleaning up existing Rust code" to each description and got 0 of 4. Possible reasons the guidance does not rule out: the sentence was appended last (P6 says lead with it), it was not imperative or pushy ("Also use when" versus "Use this skill whenever ... even if"), and six skills saying the same thing compete (P9). The corrected versions fix the first two; whether that changes the result is untested. The guidance's own test method (P10) is the right check: about 10 should-trigger prompts per skill, including "add this feature" and "clean this up", 3 runs each.
2. **P8 explains the "add a feature" zero, and the guidance offers no fix for it.** Both S4 and S5 say the agent skips skills for tasks it believes it can handle alone. Writing ordinary Rust is exactly such a task for a strong model; a greenfield spec build is complex enough to trigger, a feature request is not. No source says how to make a style or quality skill load on routine tasks; their examples are all specialized domains (PDF, Excel, BigQuery). The trial's router result (F3) is the only evidence here.
3. **Many narrow skills that should load together is not covered.** S7 says one job per skill; S6 says be distinctive; S1 shows one skill with reference files as the multi-domain pattern. None covers sibling skills that all apply to one request, or a router skill. Trial F1 and F3 suggest that for this use, narrow descriptions lose to one entry point whose subject is the whole job. Merging the six into one skill with six reference files (S1 Pattern 2) is the guidance-backed version of that and was not tried.
4. **Load once, at the start.** S2 confirms the body is not re-read, but no source says the agent consults the skill list only once. Trial F2 established that it does. The guidance's "when X happens" phrasing (S1 "when working with PDF files") reads as if it could fire mid-task; for a coding agent it in practice means "when the request mentions X". This is the gap that makes the current edit-level triggers dead text.
5. **Third person versus imperative.** S1 says third person; S4 and S6 say imperative "Use this skill when". Both forbid "I" and "you" and their examples combine a capability clause with "Use when", so this is wording, not substance. The corrected versions lead with the imperative trigger because of P6.
6. **Mechanisms outside the description that the guidance mentions but the trial did not test.** Claude Code's `when_to_use` field (appended to the description, Claude Code only, so other agents ignore it) and `paths` (glob patterns for files that activate the skill; S2 does not say whether this fires mid-session, so it is unverified). S2 also recommends a hook for rules that must hold every time, which is the documented answer to "never re-checked mid-task".
7. **Listing budget.** Not a problem today (all descriptions are delivered in full, per the trial), but in a Claude Code setup with many skills, descriptions are dropped starting with the least invoked (S2), which would hit rarely triggered skills like these first.
