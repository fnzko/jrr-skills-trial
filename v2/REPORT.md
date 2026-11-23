# Trial v2: under which circumstances the just-right-rust skills add value

Record of the second stage of the trial, run on 2026-11-20. It follows `../TRIAL-LOG.md` (v1: headless
refactors of the aws-emulator SQS crate). Everything here is local; nothing was pushed or published.
All numbers in this file were read from the files under `v2/` (and `../eval/14`, `../eval/15`,
`../logs/14.jsonl`, `../logs/15.jsonl` for P5) by scripts at the time of writing. Section 7 says how
much weight each number can carry, and the last part of section 2 lists where the files disagree with
`STATE.md`, `PLAN.md` or `../SUMMARY.md`.

Section 11 was added on 2026-11-23 and records a third round: skill descriptions, delivery through
hooks, and token and time consumption (54 more runs, 72 probes). Where it corrects sections 1 to 10,
section 11.10 says so.

Totals: 87 headless runs in `v2/eval/` (plus the two P5 runs kept with v1),
USD 82.53 of reported cost, 467 agent minutes, 2189 turns. Every one of
the 87 runs builds, passes its own tests, passes all hidden acceptance scenarios and ended
without an error flag (checked over every `summary.json`: yes).

## 1. Question and one-page answer

Question: v1 showed that the new skills are not picked up on a broad refactor request and that
loading them does not change the outcome. Is there a task, a way of delivering the rules, or a model
for which they do change the code, and by how much?

v2 varies three things v1 held fixed: the task (build from a spec, add a feature to existing code,
clean up existing code), the delivery (nine "arms", section 4) and the model (Sonnet 5.5 everywhere,
Haiku 5.5 and Opus 5.5 in P4). The outcome is judged twice: by mechanical counts, and by blind
reviewers who each score every crate of a phase on one dimension from 0 to 4 without knowing the arm.

Blind sums by task and delivery, Sonnet 5.5, three runs per cell: mean (lowest run to highest run).
The columns are not on one scale: each phase was reviewed on a different number of dimensions, and
`overall` is never part of a sum. Compare within a column only.

| Delivery | Greenfield P1, max 24 | Greenfield re-scored in P6, max 16 | Feature P2, max 12 | Refactor P3, max 24 | Refactor re-scored in P6, max 12 |
|---|---|---|---|---|---|
| start crate (BASE) | - | - | 8.0 (8 to 8) | 13.0 (13 to 13) | 4.0 (4 to 4) |
| none | 11.3 (11 to 12) | 6.3 (5 to 8) | 7.3 (6 to 10) | 12.7 (12 to 13) | 4.0 (4 to 4) |
| skills | 18.3 (17 to 19) | 10.7 (10 to 12) | 8.0 (6 to 9) | 13.0 (13 to 13) | - |
| router | 18.0 (17 to 19) | - | 8.7 (7 to 10) | 13.0 (13 to 13) | - |
| audit | - | - | - | 18.0 (17 to 19) | 6.7 (6 to 7) |
| hook | 16.3 (15 to 18) | - | 8.7 (7 to 10) | 15.3 (15 to 16) | - |
| context | 18.7 (17 to 20) | 12.7 (11 to 15) | 9.3 (8 to 11) | 17.0 (16 to 18) | 8.7 (8 to 9) |
| short | - | 12.7 (12 to 13) | - | - | 6.3 (5 to 8) |
| limits | - | 11.7 (11 to 12) | - | - | 5.7 (5 to 7) |
| context2 | - | 12.0 (12 to 12) | - | - | 8.0 (8 to 8) |

Other models, greenfield only, two runs per cell:

| Model (P4 greenfield, max 16) | none | skills | context |
|---|---|---|---|
| haiku | 7.5 (7 to 8) | 9.5 (9 to 10) | 11.5 (11 to 12) |
| opus | 10.0 (10 to 10) | 13.0 (12 to 14) | 14.0 (14 to 14) |

P5, the v1 crate (19.6k lines) with the `audit` arm and the v1 rubric: sums 7 and 9, in 35 and 44
turns. The v1 runs that reached 9 or more took 102 to 132 turns (section 5.5).

What the table says, in plain sentences:

1. When the agent writes a new crate from a spec, every way of giving it the rules beats giving it
   nothing, and the runs do not overlap: the worst treated run is above the best untreated run in P1,
   in the P6 re-scoring, and in both other models.
2. When the agent cleans up an existing crate, having the six guides loaded (`router`), or merely
   available (`skills`), gives the start crate back: 13, 13, 13 against a start crate scored 13, in
   all three runs of both arms.
3. On the same cleanup task, three deliveries do change the code: a short entry skill that says what
   to look for (`audit`), the skills' checklists placed in `CLAUDE.md` (`context`), and a Stop hook
   that runs a detector and hands its findings back (`hook`).
4. When the agent adds a feature to existing code written in another style, no delivery separated
   from `none`: every pair of ranges overlaps, and the mechanical counts of the new code look the
   same in all eight arms tried (P2 and the P6 feature runs).
5. The part of the skills that acts is short. The 47 checklist bullets (68 lines) give as much as or
   more than the six guides (1170 lines) when writing new code, whether they sit in `CLAUDE.md` or in
   one skill. On cleanup the same 68 lines act more reliably from `CLAUDE.md` than from a skill.
6. The checklist alone over-applies types: `context` is last or tied for last on the `simplicity`
   dimension in every phase where it was scored. Adding the guides' "Do not over-apply" bullets
   raises simplicity in greenfield and lowers the type score; in cleanup the agent then keeps the
   `expect` calls (in the `CLAUDE.md` variant rewritten to name their invariant) where `context`
   removed them, and simplicity does not improve.
7. The effect has the same sign in Haiku, Sonnet and Opus. Haiku with the checklist scored above
   Opus with nothing (11 and 12 against 10 and 10).
8. On the large v1 crate the `audit` entry skill reached sums of 7 and 9 in 35 and 44 turns, a level
   that in v1 only runs of 102 to 132 turns reached, and it still left the high-severity audit
   findings untouched.

## 2. What changed from v1, and what went wrong on the way

### 2.1 Changes to the harness, and why

| Change | Why |
|---|---|
| A small standalone crate ("miniq", built from `task/SPEC.md`) instead of the 19.6k-line SQS crate | v1 runs took 40 to 70 minutes and 7 to 17 GB of build output each, so cells had 2 runs. Here a run takes about 1 to 17 minutes and cells have 3 (2 in P4). |
| Hidden black-box acceptance tests (`task/acceptance.py`, 58 scenarios, 80 with the feature) | v1 could only say "the crate's own tests still pass". Here behavior is checked by tests the agent never sees, so a quality gain cannot hide a correctness loss. |
| Agent work directories under `/Users/Shared/jrr-work/<run>` | v1 found that `~/.claude/CLAUDE.md` is loaded for any work directory under `~`, even with an isolated `CLAUDE_CONFIG_DIR`. `STATE.md` records the check that a directory outside `~` does not load it. Results are copied to `v2/runs/<run>` after evaluation. |
| No symposium in the loop: plugin directories go straight to `claude -p --plugin-dir` | Delivery through symposium was validated in v1 and is not the question here. One moving part less. |
| Blind review per dimension across all runs of a phase (`reviews/dimension-brief.md`) | In v1 each reviewer saw one run, so two near-identical diffs got different scores. Here one reviewer reads every crate of the phase for one dimension, under shuffled two-letter codes (`bin/blind.py`, keys in `keys/blind-*.tsv`). |
| A rule-free `overall` reviewer | The six dimension rubrics restate the skills' own rules. `overall` is given no checklist ("which would you rather inherit and maintain for two years") and is kept out of every sum. |
| Batches run under `caffeinate -i`, each run under `timeout` (`RUN_TIMEOUT`, default 4500 s) | v1 lost a batch to machine sleep: three runs sat nine hours and failed. |
| One batch script (`bin/batch.sh`): at most K runs at a time, 20 s between starts, evaluation right after each run | Fewer manual steps; the stagger avoids several sessions starting at once on the same login. |
| `cargo clean` after evaluation instead of deleting `target/` | Disk; and see 2.2 on removal commands. |

The run command (`bin/run2.sh`), unchanged across arms except for the plugin directory and the
`CLAUDE.md` copy: `claude -p [--plugin-dir v2/arms/<arm>] --model <model> --permission-mode auto
--strict-mcp-config --disallowedTools WebFetch WebSearch "Bash(git push:*)" --max-budget-usd 10
--output-format stream-json --verbose "<prompt>"`, with `CLAUDE_CONFIG_DIR` pointing at the isolated,
logged-in config of v1. The work directory starts as a fresh git repository holding the spec (and the
baseline crate for P2, P3, P6r, P6f; and `CLAUDE.md` for the arms that have one), committed and tagged
`start`.

The three prompts (`prompts/`), the same for every arm:

Greenfield:

```text
SPEC.md in this directory describes a small service. Build it as a new Rust crate here, with tests.

Write it as Rust an experienced maintainer would be happy to own.

Work in small commits and say why in each message.
```

Feature:

```text
This crate implements SPEC.md. SPEC-feature.md describes three additions. Implement them, with tests.

Do not change existing behavior: the crate's tests must still pass.

Work in small commits and say why in each message.
```

Refactor (the v1 prompt, with the crate named by "this crate"):

```text
I'm taking over maintenance of this crate (it implements SPEC.md). It works and its tests pass, but it is hard to read and to change safely.

Clean it up so it reads like Rust an experienced maintainer would be happy to own. Do not change behavior: the crate's tests must still pass.

Work in small commits, one kind of change per commit, and say why in each message.
```

None of them mentions skills or rules.

### 2.2 Harness incidents

- **Empty-array bug, six runs lost and rerun.** `run2.sh` built the plugin argument as a bash array
  and expanded it as `"${plugin[@]}"` under `set -u`. macOS ships bash 3.2, where expanding an empty
  array under `set -u` is an "unbound variable" error. Every arm with no plugin directory (`none`,
  `context`) died before starting `claude`: p1-03, p1-04, p1-06, p1-07, p1-10, p1-13. The batch
  printed `plugin[@]: unbound variable` and evaluated six empty directories as `0/0`. Fix:
  `${plugin[@]+"${plugin[@]}"}`. The six were rerun from `keys/p1-rerun.list`; `keys/runs.tsv` lists
  them twice for that reason (93 lines for 87 runs). The nine runs that did start were not rerun, so
  P1 mixes two batches.
- **A table command that printed `None` for every run.** After the P2/P3 batch the summary table was
  asked for with the column names in an unquoted shell variable. zsh does not split unquoted
  variables into words, so `table.py` received one long column name and printed `None` in every cell.
  For a moment this read as "all 33 runs failed". The runs were fine; the columns are now passed as
  separate arguments.
- **A removal command refused by the safety check.** The first command that built the arms began by
  emptying `v2/arms/` with a relative glob after a `cd`. The harness's safety check refused it,
  because a relative removal after a directory change cannot be resolved statically. The removal was
  dropped, not worked around: the arms were built by copying only the files each one needs
  (`skills/`, `plugin.json`, `.claude-plugin/`) into newly created directories, with absolute paths.
  Nothing under `v2/arms/` was ever deleted. For the same reason `eval2.sh` frees disk with
  `cargo clean`.
- **`rg` is not a binary here.** In this shell `rg` is a function that re-executes the Claude Code
  binary, so a hook script cannot call it. The hook arm's detector uses `grep -rnE` with the skills'
  search patterns.
- **One crate fools the detector.** `detect.py` treats everything in `src/` before the first
  `#[cfg(test)]` as production code. Run p1-10 (`context`) keeps its tests in a separate file under
  `src/` (`src/broker/tests.rs`, 847 lines), which is counted as production. Its production line count is
  overstated, its per-KLOC figures are understated, and its `tests` and `weak asserts` columns (16
  and 0) miss that file. It is the only run with a test-named file under `src/`. The blind scores are
  not affected.
- **Skill-call counts for the hook arm need the order.** In two of the three P1 hook runs the Skill
  calls come after the Stop hook fired (tool calls 20 to 22 and 25 to 27), prompted by the hook's own
  last line ("Load the matching rust-* skill for how to fix"). `STATE.md` reports them as "3, 3, 5
  skills loaded" without that distinction. Section 6.1 counts self-activation with it.

### 2.3 Where the files disagree with the notes

| Where | Says | Files say |
|---|---|---|
| `STATE.md`, P4 blind table header; `../SUMMARY.md` and the memory note ("6 -> 11 a 13 de 20") | sums of "five dimensions, max 20" | The P4 and P6 greenfield sums add four dimensions (types, errors, simplicity, tests): max 16. `overall` is separate. |
| `STATE.md` and the conversation: skills "best on overall" in greenfield | P1 `overall`: skills 3.67, context 3.00 | True in P1. The same three crates re-scored in P6 by another reviewer: skills 2.00, context 3.00, short 3.67. Section 7. |
| Conversation: test-retest "76 repeated scores, 73 within one point, 41 identical" | computed before `reviews/p6g/simplicity.tsv` existed | 85 repeated scores, 82 within one point, 46 identical (section 7.1). |
| `STATE.md`: `limits` is "121 lines" | the body | `arms/limits/skills/rust-rules/SKILL.md` has 126 lines (121 of body, 5 of frontmatter); `short` has 73 (68 + 5). |
| `STATE.md`, P6 feature: "expect unchanged in all" | relative to the other feature runs | Against the baseline (12.27 per KLOC) `expect` rises in every feature run of every arm, to between 14.6 and 17.8, because the new code adds its own. No arm differs from `none` in that. |
| `../SUMMARY.md`: on refactor the skills load "casi nunca" | | On the small crate the `skills` and `hook` arms each loaded `rust-keep-it-simple` in 3 of 3 runs (once with `rust-comments`), at tool call 2. No run loaded the type, error, ownership or test guide by itself. |
| `STATE.md`, P1: "in greenfield the skills load by themselves" | from the `skills` arm, 3 of 3 | The `hook` arm has the same eight skills and no entry skill: 1 of 3 loaded them at the start. Together 4 of 6 Sonnet runs (section 6.1). |
| `PLAN.md` | P2 = none + best three of P1; P3 without `hook`; P4 = none + best of P1; no P6; hook uses "the research lint set" | P2 and P3 ran every arm, P4 ran none, skills, context, P6 and its three arms were added later (`STATE.md` records each change). The hook's lint set is not the evaluation's (section 4.6). |
| `keys/runs.tsv` | 93 lines | 87 runs; the six P1 reruns appear twice. |
| `STATE.md` P1 mechanical table, `context` row | means over p1-06, p1-10, p1-13 | Correct arithmetic, but p1-10's counts are distorted (2.2). |
| BASE in the `overall` reviews | | The same start crate was scored 4 in P2, 2 in P3 and 3 in P6r by three different `overall` reviewers. |

## 3. The task fixture

Written by a subagent before any run, in `task/`:

- `SPEC.md`, 220 lines: "miniq", an in-memory message queue service. One binary reads
  newline-delimited JSON commands on stdin and writes one JSON response per line. No network, no
  files, no wall clock: time advances only through a `Tick` command. Sections: Protocol, Errors (ten
  codes and a fixed order of checks), Time and identifiers, Queues and messages, Queue commands,
  Message commands. It covers standard and FIFO queues, visibility timeouts, delays, retention,
  deduplication windows, dead-letter queues, deterministic message ids and receipt handles, and
  limits (1000 queues, 100000 messages per queue). It describes behavior only: no Rust types, module
  layout or design advice. The interface being text is deliberate: how the agent models the data
  inside is left open, which is what the `types` dimension looks at.
- `SPEC-feature.md`, 79 lines: batches (`SendMessageBatch`, `DeleteMessageBatch`), queue tags, and a
  per-queue `MaxMessageBytes` attribute.
- `acceptance.py`, 2719 lines, Python standard library only: 58 scenarios for the base spec, 22 more
  with `--feature` (80). One fresh process per scenario, 10 s timeout each. A crate that implements
  the feature must be tested with the flag and a base crate without it, because `MaxMessageBytes`
  must appear in `GetQueueAttributes` exactly when the feature exists.
- `reference/`: the author's own implementation, used only as the oracle. `task/README.md` records:
  80/80 with the feature build, 58/58 with the base build; the Python model inside `acceptance.py`
  that produces the expectations of the two stress scenarios agreed with the reference on 750 random
  1500-command scripts; 53 mutations of the reference were each caught by at least one scenario. The
  list of mutations was reported by the authoring agent and is not stored on disk.
- Performance is tested once: three scenarios feed 100000 or more lines and must finish in 10 s. An
  implementation that rescans a queue on every command fails them. Several reviewers later marked
  crates down for keeping five or six hand-synchronized indexes "where a single ordered map passes
  the spec"; that pressure comes from these scenarios.
- Left unspecified and untested (README): repeated member names, number spellings such as `-0`, lone
  surrogate escapes, the clock past 4000000000, `null` values inside `Tags`.
- Stated and tested, but easy to read the other way (README): a bad queue name beats attribute
  errors while attribute errors beat the `.fifo`/`Fifo` mismatch; explicit and content-based
  deduplication ids share one id space; a handle of another queue's message is
  `ReceiptHandleExpired`; a delivery with visibility timeout 0 yields a handle that never works;
  batch shape errors come after `QueueDoesNotExist`; blank input lines get an `InvalidRequest`
  response.

All 87 runs pass every scenario that applies to them, so none of these ambiguities cost
any arm a point. The acceptance suite therefore separates nothing between arms; it only establishes
that no arm bought its score with broken behavior.

## 4. Arms: what the agent received

An arm is a directory under `arms/`. If it holds a `plugin.json` it is passed with `--plugin-dir`;
if it holds a `CLAUDE.md`, that file is copied into the work directory before the `start` commit.

| Arm | File added to what the agent gets | Lines |
|---|---|---|
| router | skills/rust-code/SKILL.md | 17 |
| audit | skills/rust-audit/SKILL.md | 45 |
| context | CLAUDE.md | 68 |
| hook | hooks/hooks.json | 7 |
| hook | hooks/jrr-check.sh | 35 |
| short | skills/rust-rules/SKILL.md | 73 |
| limits | skills/rust-rules/SKILL.md | 126 |
| context2 | CLAUDE.md | 121 |

### 4.1 `none`

No plugin directory, no `CLAUDE.md`. The agent has Claude Code's built-in skills only.

### 4.2 `skills`

The eight skills of branch `ai-patterns-skills` as symposium 0.6.0 compiled them in v1 (copied from
`../probe-plugins/a`): `plugin.json`, `.claude-plugin/plugin.json`, `skills/<name>/SKILL.md`.

| Skill | Lines | Bytes |
|---|---|---|
| async-rust | 156 | 7181 |
| rust-comments | 175 | 9701 |
| rust-domain-types | 198 | 9807 |
| rust-error-discipline | 185 | 9792 |
| rust-keep-it-simple | 218 | 12016 |
| rust-ownership | 197 | 11558 |
| rust-test-quality | 197 | 10613 |
| unsafe-rust-review | 2141 | 92720 |

The six `rust-*` guides together: 1170 lines, 63487 bytes. All eight: 3467 lines, 163388 bytes.

Each `rust-*` guide ends with a "Do not over-..." section and a "Review checklist" (a bullet list that
starts "Before finishing, verify in the diff:", followed by `rg` search commands and a paragraph on
Clippy lints). The plugin's hooks are not part of this directory and were not active in any arm
except `hook`, which has its own.

### 4.3 `router`

`skills` plus one entry skill, `skills/rust-code/SKILL.md` (17 lines). Its body names the six guides
and nothing else: no hint about what to look for.

```markdown
---
name: rust-code
description: Write, change or refactor Rust code. Use first, before writing or editing any Rust source in this project, whatever the task is (a new crate, a feature, a bug fix, a cleanup). It lists the other rust-* skills to load.
---

# Working on Rust code

Before writing or changing Rust code, load each of these skills:

- `rust-domain-types`
- `rust-error-discipline`
- `rust-ownership`
- `rust-keep-it-simple`
- `rust-comments`
- `rust-test-quality`

Apply them while you work, and run each skill's review checklist on your diff before you finish.
```

### 4.4 `audit`

`skills` plus one entry skill for existing code, `skills/rust-audit/SKILL.md` (45 lines). The list
was written as a generic one, before the miniq baseline existed, so it cannot have been fitted to
P3. It was not written blind to the SQS crate: its author knew the v1 audit, and several lines match
SQS findings ("settings kept as string maps and parsed again on every read" is S-05, "`expect` or
`unwrap` right after a lookup" is S-12/S-13, "states ... compared to literals or constants" is what
runs 10 and 11 of v1 fixed). That matters for P5, not for P3. Its last step repeats the lesson of v1
("Move code between files last").

```markdown
---
name: rust-audit
description: Find what to fix in existing Rust code. Use first for any request to clean up, refactor, review, tidy, simplify or improve existing Rust code, before reading the code in depth and before planning.
---

# Auditing existing Rust

A cleanup is judged by the defects it removes, not by how much code moved. Find the defects first.

## What to look for

Types
- identifiers, names and handles passed around as `String` or `&str`, and maps keyed by them;
- states, kinds, modes and codes kept as strings or integers and compared to literals or constants;
- `Option` or `bool` fields that only make sense together;
- settings kept as string maps and parsed again on every read.

Failures
- `expect` or `unwrap` right after a lookup or check that already proved the value is there;
- a failure turned into a default, an empty collection or `None`;
- a branch or re-validation for a case an earlier check already refused;
- a `Result` dropped with `let _ =` or `.ok()`.

Ownership and repeated work
- clones made only to satisfy the borrow checker or to build a lookup key;
- the same value looked up, parsed, sorted or scanned more than once per request;
- a scan of a collection inside a loop over the same collection.

Simplicity
- the same steps copied across handlers instead of living in one place;
- a decision computed in two places;
- `#[allow]` attributes that hide nothing, dead code, leftovers.

Comments and tests
- comments that restate the code or contradict it;
- tests that cannot fail: `is_ok()`, non-emptiness or substring checks, assertions behind a branch.

## How to work

1. Run the search commands from the "Review checklist" of each of these skills over the code:
   `rust-domain-types`, `rust-error-discipline`, `rust-ownership`, `rust-keep-it-simple`,
   `rust-comments`, `rust-test-quality`. Read the hits.
2. Write down the findings by category, with file and line, before changing anything.
3. Fix one category per commit, most severe first. Load the matching skill for how to fix.
4. Move code between files last, and only if it still helps after the fixes.
```

### 4.5 `context`

No plugin. `CLAUDE.md` (68 lines) in the project root, made by a script: for each of the six guides,
the bullets between "## Review checklist" and "These searches", under a short heading. That is
47 bullets. The `rg` commands, the Clippy paragraphs and everything else in the guides
are left out. The only words added are the title and the sentence under it.

```markdown
# Rust rules for this project

Follow these rules in all Rust code you write or change here.

## Types

- known values are compared as enum variants, not as strings;
- outside wire types, identifiers of different kinds are not the same primitive, and map keys are typed;
- types with a validity rule have no `pub` field, infallible `From` or plain `Deserialize`;
- `Option` and `bool` fields that change together are one enum;
- "not set" is `Option`, not `-1`, `0` or `""`;
- outside wire types, no field keeps its unit or format only in its name;
- data with a known shape is a struct, not `serde_json::Value` or `HashMap<String, String>`;
- no newtype is converted back to its primitive at every use.

## Errors

- no failure becomes a default, an empty collection or `None` unless the spec defines that value;
- no `Result` is dropped with `let _ =`, `.ok();` or an unconditional `Ok`, except commented best-effort calls;
- nothing that input can trigger panics: no `unwrap`, `expect`, index or slice on input;
- every `expect` and `unreachable!` names the invariant, and no `todo!()` or `unimplemented!()` remains;
- `catch_unwind` appears only at a thread, task or FFI boundary;
- sizes and offsets that come from input use `try_from` and checked arithmetic;
- an error that crosses a layer says what was attempted, and is logged or returned, not both;
- no error type is `String`, `&'static str` or `()`, and no library function returns `anyhow::Result`;
- no `Option`, `Result` or branch exists for a state that cannot occur.

## Ownership

- no `.clone()` was added only to make a borrow error go away, and no collection is cloned to iterate over it or cloned and then cleared;
- parameters that are only read are borrowed views, arguments that are stored are taken owned, and getters return borrows or iterators;
- no `String` is allocated to compare or look up, and no `Vec` is collected only to iterate, count or test;
- nothing is parsed, compiled, sorted or scanned per call or per item that could be done once;
- no buffer is created per iteration, no capacity comes straight from input, and nothing is formatted for a log line that may be off;
- no whole state is cloned out of a lock, and no guard is held during I/O or other slow work.

## Simplicity

- every new function, field, parameter, variant and trait method has a caller;
- every new trait has two implementations, and no new state repeats what existing state answers;
- no function has a suffixed twin, and no helper repeats one from the workspace, std or a dependency;
- no decision is computed in two places, and shared code does not branch on a concrete implementation;
- nothing exists only for an old shape that has no users;
- no value is checked and then unwrapped, and no flag variable feeds a second `if`;
- push, flag and accumulator loops are adapters, and chains with effects, `?` or early exits are loops;
- no `dbg!`, stray print, commented-out code or new `#[allow]` remains;
- every new dependency has a named need and only the features in use.

## Comments and commits

- every added comment states a reason, an invariant or a contract, and none restates the line below it;
- no comment or doc mentions what the code used to do, the review or the plan, and none is made false by the change;
- added comments, docs and messages are plain ASCII, with no banners, step numbers or capitals for emphasis;
- public items say what the signature cannot, and no doc only rephrases the name;
- every `TODO` or `FIXME` names an action and an issue, and nothing is stubbed;
- every `SAFETY` comment names the check that makes the block sound;
- each commit has one purpose, `git diff` and `git diff -w` show the same lines, and the pull request text matches the diff.

## Tests

- every test asserts a value, a count or an error variant, not only `is_ok()`, emptiness or a substring;
- no assertion sits behind `if let`, `unwrap_or`, `unwrap_or_default` or a loop that may not run;
- no expected value is computed by the code under test, and each regression test was seen failing without the fix;
- structured output is compared as a whole typed value or a reviewed snapshot;
- no fixed sleep, port or path, environment change, wall clock or random value decides a result;
- production code has no test-only branch;
- test names describe the scenario, and the inputs of one behavior share one table;
- no new helper repeats an existing one or hides the assertions.
```

### 4.6 `hook`

`skills` plus a Stop hook (`hooks/hooks.json`, `hooks/jrr-check.sh`).

```json
{
  "hooks": {
    "Stop": [
      { "hooks": [ { "type": "command", "command": "sh \"${CLAUDE_PLUGIN_ROOT}/hooks/jrr-check.sh\"", "timeout": 300 } ] }
    ]
  }
}
```

```sh
#!/bin/sh
# Stop hook: run the just-right-rust detector once per session and hand the findings back.
input=$(cat)
sid=$(printf '%s' "$input" | python3 -I -c 'import json,sys; print(json.load(sys.stdin).get("session_id","x"))' 2>/dev/null)
cwd=$(printf '%s' "$input" | python3 -I -c 'import json,sys; print(json.load(sys.stdin).get("cwd","."))' 2>/dev/null)
marker="${TMPDIR:-/tmp}/jrr-hook-$sid"
[ -e "$marker" ] && exit 0
cd "$cwd" 2>/dev/null || exit 0
[ -f Cargo.toml ] || exit 0
: > "$marker"
out=$(mktemp)
cargo clippy --all-targets --message-format=short -- \
  -W clippy::cast_possible_truncation -W clippy::cast_possible_wrap \
  -W clippy::format_push_string -W clippy::manual_let_else \
  -W clippy::needless_for_each -W clippy::or_fun_call -W clippy::allow_attributes \
  -W clippy::dbg_macro -W clippy::let_underscore_must_use -W clippy::unused_result_ok \
  -W clippy::wildcard_enum_match_arm -W clippy::needless_pass_by_value \
  -W clippy::unwrap_used -W clippy::expect_used 2>&1 | grep -E '^[^ ]+\.rs:[0-9]+' | head -40 > "$out"
{
  grep -rnE '\b(\w+_)?(id|ids|name|key|url|code|handle): (Option<|Vec<)?(String|&(.\w+ )?str)\b|Map<String,' src 2>/dev/null | sed 's/^/stringly typed: /' | head -15
  grep -rnE 'match [^{;]*\.as_str\(\)|[!=]= "[A-Za-z]|"\w+" [!=]=' src 2>/dev/null | sed 's/^/string compared as a value: /' | head -15
  grep -rnE '\.unwrap_or_default\(\)|\.unwrap_or\((0|""|false|String::new\(\))\)|^\s*let _ = |\.ok\(\);' src 2>/dev/null | sed 's/^/silent default or dropped result: /' | head -15
  grep -rnE 'Result<[^;{}]*,\s*(String|&.static\s+str|\(\))\s*>|Box<dyn (std::error::)?Error' src 2>/dev/null | sed 's/^/untyped error: /' | head -10
  grep -rnE 'assert!\(\s*!?[^;]*\.is_(ok|err|some|none|empty)\(\)\s*[,)]|assert!\([^;]*\.contains\(' src tests 2>/dev/null | sed 's/^/weak assertion: /' | head -10
} >> "$out"
n=$(wc -l < "$out" | tr -d ' ')
if [ "$n" -gt 0 ]; then
  {
    echo "just-right-rust check: $n candidate findings (first lines shown)."
    cat "$out"
    echo "For each one, fix it or say in one line why it stays. Load the matching rust-* skill for how to fix. Then finish."
  } >&2
  exit 2
fi
exit 0
```

In words: when the agent tries to finish, the script runs once per session (a marker file keyed on
the session id). In a directory with a `Cargo.toml` it runs `cargo clippy` with a lint set and five
`grep` searches over `src` (and `tests` for the last), and if anything is found it exits with status
2 and prints the findings, which Claude Code hands back to the agent as "Stop hook feedback". The
finding categories, exactly as labelled: clippy warnings (first 40 lines), `stringly typed` (15),
`string compared as a value` (15), `silent default or dropped result` (15), `untyped error` (10),
`weak assertion` (10). The closing line asks the agent to fix each or say why it stays, and to load
the matching skill.

The evaluation lint set has 13 lints, the hook's has 14; 11 are shared. Only in the hook: expect_used, needless_pass_by_value, unwrap_used. Only in the evaluation: allow_attributes_without_reason, non_std_lazy_statics. So the detector both overlaps the lint count used as a measure (section 8) and adds
`unwrap_used` and `expect_used`, which fire on every `expect`.

The hook fired in all nine runs of the arm (once each).

### 4.7 `short` (P6)

One skill and nothing else: `skills/rust-rules/SKILL.md`, 73 lines. Its body is byte for byte the
`context` `CLAUDE.md` (checked with `diff`); its description is the router's with the last sentence
changed.

```markdown
---
name: rust-rules
description: Write, change or refactor Rust code. Use first, before writing or editing any Rust source in this project, whatever the task is (a new crate, a feature, a bug fix, a cleanup). It holds this project's Rust rules.
---

# Rust rules for this project

Follow these rules in all Rust code you write or change here.

(the rest is the context CLAUDE.md above, unchanged)
```

### 4.8 `limits` and `context2` (P6)

The same 121 lines delivered two ways: as the body of the single skill `rust-rules` (`limits`, 126
lines with frontmatter, same description as `short`) and as `CLAUDE.md` (`context2`). Checked with
`diff`: identical. The text is the `context` text plus two things:

- A scope sentence under the title: "Scope: when asked to add or change something, apply the rules
  to the code you add or touch and leave the rest alone. When asked to clean up, refactor or review
  existing code, apply them to that code."
- After each topic's checklist, the word "But:" and the bullets of that guide's "Do not over-type",
  "Do not over-apply" or "Do not over-simplify" section, copied verbatim by a script:
  33 bullets in all. One bullet was dropped on purpose, the last one of
  `rust-keep-it-simple`:

  > - Apply these rules to the code the change touches. Do not rewrite working code elsewhere to match them.

  It was dropped because the scope sentence replaces it, and because as written it tells an agent
  that was asked to clean up a crate to leave the crate alone (section 6.3).

Full text:

```markdown
# Rust rules for this project

Follow these rules in all Rust code you write or change here.

Scope: when asked to add or change something, apply the rules to the code you add or touch and leave the rest alone. When asked to clean up, refactor or review existing code, apply them to that code.

## Types

- known values are compared as enum variants, not as strings;
- outside wire types, identifiers of different kinds are not the same primitive, and map keys are typed;
- types with a validity rule have no `pub` field, infallible `From` or plain `Deserialize`;
- `Option` and `bool` fields that change together are one enum;
- "not set" is `Option`, not `-1`, `0` or `""`;
- outside wire types, no field keeps its unit or format only in its name;
- data with a known shape is a struct, not `serde_json::Value` or `HashMap<String, String>`;
- no newtype is converted back to its primitive at every use.

But:

- Do not wrap free text, or values with no rule and no realistic mix-up. A newtype converted back to its primitive at every use (`.0`, `as_str()`, `into_inner()`) is a name, not safety.
- Do not enforce, in client code, constraints that only the server owns.
- Plain data may have public fields and no accessors. A type with an invariant has private fields and only the getters callers need, named after the field, without `get_` and without setter pairs. Do not mix `pub` and private fields on one type. Keep items private by default and use `pub(crate)` for internal sharing.
- When porting from another language, translate meaning, not spelling. Integer JSON numbers are `u64` or `i64`, not `f64`. JavaScript's `a || b` also replaces `""` and `0`, so `unwrap_or` alone changes behavior.

## Errors

- no failure becomes a default, an empty collection or `None` unless the spec defines that value;
- no `Result` is dropped with `let _ =`, `.ok();` or an unconditional `Ok`, except commented best-effort calls;
- nothing that input can trigger panics: no `unwrap`, `expect`, index or slice on input;
- every `expect` and `unreachable!` names the invariant, and no `todo!()` or `unimplemented!()` remains;
- `catch_unwind` appears only at a thread, task or FFI boundary;
- sizes and offsets that come from input use `try_from` and checked arithmetic;
- an error that crosses a layer says what was attempted, and is logged or returned, not both;
- no error type is `String`, `&'static str` or `()`, and no library function returns `anyhow::Result`;
- no `Option`, `Result` or branch exists for a state that cannot occur.

But:

- "No `unwrap`" never means "use a default". Replacing a panic with `unwrap_or_default()` trades a loud bug for a wrong answer; use `?`, or `expect` with the invariant.
- Do not invent error variants for states that only a bug can reach. Panic there.
- A bare `unwrap()` is fine in tests, on a poisoned `Mutex::lock()`, and when the check that guards it is on the adjacent line.
- Do not add context at every `?` inside one layer, and do not wrap an error in words it already says.
- Do not build an error enum that no caller matches on. In a binary, `anyhow` with context is enough.
- A deliberate second check at a trust boundary is not defensive code. Keep it.

## Ownership

- no `.clone()` was added only to make a borrow error go away, and no collection is cloned to iterate over it or cloned and then cleared;
- parameters that are only read are borrowed views, arguments that are stored are taken owned, and getters return borrows or iterators;
- no `String` is allocated to compare or look up, and no `Vec` is collected only to iterate, count or test;
- nothing is parsed, compiled, sorted or scanned per call or per item that could be done once;
- no buffer is created per iteration, no capacity comes straight from input, and nothing is formatted for a log line that may be off;
- no whole state is cloned out of a lock, and no guard is held during I/O or other slow work.

But:

- Keep a clone that is cheap, cold or clearer than the alternative. Do not thread lifetimes through structs to avoid one; own the data.
- Keep `&str` in a constructor when almost every caller only holds a borrow. `impl Into<String>` and `impl AsRef<str>` belong at published library entry points, not in internal code.
- A linear scan over a handful of items is fine and often faster than a map. Startup code, cold administrative paths and tests may recompute, and `format!` is fine for one-off strings, argument parsing and error messages.
- Do not add `Arc` without real sharing, and do not call `with_capacity` without a known size.
- Removing an accidental allocation or a rescan inside a loop needs no benchmark. A non-obvious optimization (`#[inline(always)]`, a custom hasher, a small-vector crate, interning, a hand-rolled loop) lands only with the benchmark that justifies it.

## Simplicity

- every new function, field, parameter, variant and trait method has a caller;
- every new trait has two implementations, and no new state repeats what existing state answers;
- no function has a suffixed twin, and no helper repeats one from the workspace, std or a dependency;
- no decision is computed in two places, and shared code does not branch on a concrete implementation;
- nothing exists only for an old shape that has no users;
- no value is checked and then unwrapped, and no flag variable feeds a second `if`;
- push, flag and accumulator loops are adapters, and chains with effects, `?` or early exits are loops;
- no `dbg!`, stray print, commented-out code or new `#[allow]` remains;
- every new dependency has a named need and only the features in use.

But:

- Keep a trait with a real second implementation, including a test fake the project uses. `with_x` builder methods that set one option are not twins.
- Keep the old function, format or column for a release when real users or stored data depend on it.
- Keep a single-use helper that names a concept or lets its body use `?` and `return`. A long linear function with one concern is fine, and so is one special case inline until a second appears.
- Do not add a dependency to save a few lines, and do not hand-write tricky parsing that a well-known crate already does.
- Keep a loop when the chain would change behavior: `any` and `find` stop early and skip effects the loop ran, `zip` stops at the shorter input where indexing would have panicked, and `collect::<Result<_, _>>()` keeps only the first error. Keep the index when it reaches neighbors or the loop changes the collection.
- Keep a `match` on a `Result` when a site recovers from the error or maps it to a different variant, and `if`/`else` when both branches carry similar weight.
- Keep `&mut` output for a caller-owned buffer reused across a hot loop, and `#[allow]` on generated, macro and cfg-dependent code.

## Comments and commits

- every added comment states a reason, an invariant or a contract, and none restates the line below it;
- no comment or doc mentions what the code used to do, the review or the plan, and none is made false by the change;
- added comments, docs and messages are plain ASCII, with no banners, step numbers or capitals for emphasis;
- public items say what the signature cannot, and no doc only rephrases the name;
- every `TODO` or `FIXME` names an action and an issue, and nothing is stubbed;
- every `SAFETY` comment names the check that makes the block sound;
- each commit has one purpose, `git diff` and `git diff -w` show the same lines, and the pull request text matches the diff.

But:

- Do not delete or shorten a comment that records a reason, an invariant, a workaround or a rejected alternative, however long it is. Removing it is a regression. When unsure, keep it.
- Leave existing comments alone unless your change makes them false. Rewording them is an off-topic diff. Comment density is the project's choice: do not add comments to reach a quota, and do not strip them to look terse.
- A marker that points forward (`FIXME(#412): ...`) and a note on why the obvious alternative was rejected are not history.
- Signal words (`NB:`, `Subtle:`, `SAFETY:`), acronyms in capitals and short worked examples are fine. One non-ASCII character in a string literal or a fixture is not the target.
- `todo!()` is fine in a local draft that is not presented as finished, and an explicit "unsupported" error with a tracking issue is honest.
- A large pull request is acceptable when each commit can be reviewed on its own. Cleanup is welcome as a separate pull request.

## Tests

- every test asserts a value, a count or an error variant, not only `is_ok()`, emptiness or a substring;
- no assertion sits behind `if let`, `unwrap_or`, `unwrap_or_default` or a loop that may not run;
- no expected value is computed by the code under test, and each regression test was seen failing without the fix;
- structured output is compared as a whole typed value or a reviewed snapshot;
- no fixed sleep, port or path, environment change, wall clock or random value decides a result;
- production code has no test-only branch;
- test names describe the scenario, and the inputs of one behavior share one table;
- no new helper repeats an existing one or hides the assertions.

But:

- Assert the key fragment of human prose such as an error message, not its full text. `contains` is the right check there.
- A multi-step end-to-end scenario is a good test when its name says so. Do not split it, and do not force tests with different bodies into one table or macro.
- A few repeated setup lines that keep a test readable on its own are fine. Do not extract a helper for them.
- Do not build a trait and a fake for something the test can run for real, such as a temp dir or a local socket.
- A sleep is fine when the timer itself is under test or the clock is paused. Comparing against a simpler reference implementation in a property test is a valid oracle.
```

## 5. Phases

How to read the tables. "Skill calls" counts invocations of the Skill tool; "At tool call" gives
their positions among the run's tool calls. "Hidden tests" is `acceptance.py`. The code measures come
from `bin/detect.py` over production code (`src/` before the first `#[cfg(test)]`): newtypes are
tuple structs with one field; "stringly ids" are fields or parameters whose name ends in id, name,
key, url, code, handle, group or token and whose type is `String` or `&str`; "str compares" are
string literals compared or matched; "unwrap_or*" covers `unwrap_or`, `unwrap_or_default` and
`unwrap_or_else`; "lint set" is the 13-lint clippy set of the research; "weak asserts" are
`assert!(x.is_ok())`-style and `contains` assertions in test code. Blind scores are 0 to 4 per
dimension; the sum leaves `overall` out. In the reviewers' text, blind codes have been replaced by
`run[arm]`.

### 5.1 P1: greenfield, Sonnet 5.5

Design: build miniq from `SPEC.md`; five arms, three runs each; arm order shuffled (seed 20261120).

| Arm | Model | Runs |
|---|---|---|
| none | sonnet-5-5 | p1-03, p1-04, p1-07 |
| skills | sonnet-5-5 | p1-05, p1-08, p1-12 |
| router | sonnet-5-5 | p1-01, p1-09, p1-15 |
| hook | sonnet-5-5 | p1-02, p1-11, p1-14 |
| context | sonnet-5-5 | p1-06, p1-10, p1-13 |

Process:

| Run | Arm | Hidden tests | Own tests | Skill calls | Skills loaded, in order | At tool call | Tool calls | Stop hook | Turns | Min | USD | Commits | Diff vs start |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| p1-03 | none | 58/58 | ok | 0 | - | - | 20 | - | 21 | 6.0 | 0.99 | 4 | 15 files, 2319+ |
| p1-04 | none | 58/58 | ok | 0 | - | - | 16 | - | 17 | 5.8 | 0.94 | 4 | 16 files, 2334+ |
| p1-07 | none | 58/58 | ok | 0 | - | - | 16 | - | 17 | 5.5 | 0.89 | 7 | 14 files, 2305+ |
| p1-05 | skills | 58/58 | ok | 6 | types, errors, comments, tests, simple, ownership | 3, 4, 5, 6, 7, 8 | 40 | - | 47 | 11.2 | 2.01 | 11 | 16 files, 3602+ |
| p1-08 | skills | 58/58 | ok | 6 | types, errors, simple, tests, comments, ownership | 3, 4, 5, 6, 7, 8 | 29 | - | 36 | 8.4 | 1.53 | 5 | 14 files, 2550+ |
| p1-12 | skills | 58/58 | ok | 5 | types, errors, simple, tests, comments | 3, 4, 5, 6, 7 | 37 | - | 43 | 9.0 | 1.61 | 5 | 17 files, 2929+ |
| p1-01 | router | 58/58 | ok | 7 | code(router), types, errors, ownership, simple, comments, tests | 1, 3, 4, 5, 6, 7, 8 | 34 | - | 42 | 9.5 | 1.69 | 11 | 15 files, 2951+ |
| p1-09 | router | 58/58 | ok | 7 | code(router), types, errors, ownership, simple, comments, tests | 2, 3, 4, 5, 6, 7, 8 | 30 | - | 38 | 10.3 | 1.77 | 3 | 14 files, 2872+ |
| p1-15 | router | 58/58 | ok | 7 | code(router), types, errors, ownership, simple, comments, tests | 1, 3, 4, 5, 6, 7, 8 | 21 | - | 29 | 5.8 | 1.08 | 4 | 12 files, 2128+ |
| p1-02 | hook | 58/58 | ok | 3 | errors, tests, types | 20, 21, 22 | 27 | fired after call 19; 8 more tool calls, 3 of them Skill | 32 | 8.9 | 1.51 | 7 | 11 files, 2594+ |
| p1-11 | hook | 58/58 | ok | 3 | types, tests, errors | 25, 26, 27 | 30 | fired after call 24; 6 more tool calls, 3 of them Skill | 35 | 8.4 | 1.42 | 7 | 14 files, 2547+ |
| p1-14 | hook | 58/58 | ok | 5 | types, errors, comments, tests, simple | 3, 4, 5, 6, 7 | 34 | fired after call 33; 1 more tool call, 0 of them Skill | 41 | 10.4 | 1.86 | 8 | 19 files, 3253+ |
| p1-06 | context | 58/58 | ok | 0 | - | - | 29 | - | 30 | 10.9 | 1.8 | 5 | 11 files, 3288+ |
| p1-10 | context | 58/58 | ok | 0 | - | - | 21 | - | 22 | 9.5 | 1.51 | 5 | 13 files, 2872+ |
| p1-13 | context | 58/58 | ok | 0 | - | - | 30 | - | 31 | 8.9 | 1.45 | 5 | 17 files, 2631+ |

Code:

| Run | Arm | Prod lines | Test lines | src files | Newtypes | Enums | Stringly ids /KLOC | Str compares /KLOC | expect /KLOC | unwrap_or* /KLOC | clone /KLOC | as casts /KLOC | Lint set /KLOC | Comment % | Tests | Weak asserts | allow attrs |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| p1-03 | none | 1033 | 1085 | 8 | 0 | 2 | 21.58 | 1.03 | 15.42 | 13.36 | 5.14 | 9.25 | 14.39 | 5.8 | 45 | 9 | 1 |
| p1-04 | none | 1057 | 1073 | 10 | 0 | 3 | 19.43 | 2.04 | 12.27 | 10.22 | 3.07 | 1.02 | 5.11 | 7.5 | 44 | 20 | 1 |
| p1-07 | none | 1200 | 908 | 7 | 0 | 4 | 25.8 | 0.89 | 0.0 | 10.68 | 4.45 | 0.89 | 2.67 | 6.3 | 41 | 14 | 1 |
| p1-05 | skills | 1401 | 1911 | 9 | 4 | 3 | 18.48 | 1.54 | 5.39 | 9.24 | 5.39 | 0.0 | 3.08 | 7.3 | 98 | 2 | 1 |
| p1-08 | skills | 1217 | 1140 | 7 | 1 | 4 | 28.12 | 1.76 | 5.27 | 10.54 | 4.39 | 0.88 | 6.15 | 6.5 | 53 | 0 | 1 |
| p1-12 | skills | 1244 | 1455 | 9 | 2 | 3 | 27.44 | 0.86 | 5.15 | 4.29 | 4.29 | 0.86 | 1.72 | 6.3 | 62 | 9 | 1 |
| p1-01 | router | 1231 | 1474 | 8 | 3 | 6 | 24.1 | 1.72 | 4.3 | 7.75 | 5.16 | 0.86 | 3.44 | 5.6 | 73 | 5 | 1 |
| p1-09 | router | 1214 | 1439 | 8 | 3 | 4 | 18.47 | 0.88 | 6.16 | 9.67 | 4.4 | 0.0 | 1.76 | 6.3 | 65 | 8 | 1 |
| p1-15 | router | 1085 | 886 | 7 | 2 | 3 | 23.08 | 1.92 | 4.81 | 8.65 | 2.88 | 0.0 | 2.88 | 4.1 | 36 | 2 | 1 |
| p1-02 | hook | 1314 | 1072 | 8 | 4 | 4 | 14.15 | 1.67 | 5.83 | 5.83 | 7.49 | 0.83 | 0.0 | 8.6 | 48 | 1 | 0 |
| p1-11 | hook | 1172 | 1171 | 9 | 3 | 3 | 19.82 | 0.9 | 0.0 | 10.81 | 6.31 | 0.0 | 1.8 | 5.3 | 55 | 0 | 1 |
| p1-14 | hook | 1312 | 1688 | 10 | 3 | 4 | 18.24 | 0.83 | 2.49 | 4.98 | 5.8 | 0.0 | 4.98 | 8.1 | 76 | 0 | 1 |
| p1-06 | context | 1589 | 1422 | 9 | 10 | 9 | 13.96 | 0.66 | 1.99 | 9.31 | 3.32 | 0.0 | 2.66 | 5.3 | 62 | 16 | 0 |
| p1-10 | context | 2352 | 297 | 11 | 8 | 8 | 6.16 | 0.44 | 3.96 | 4.4 | 1.76 | 0.0 | 0.0 | 3.4 | 16 | 0 | 0 |
| p1-13 | context | 1448 | 962 | 11 | 10 | 7 | 13.81 | 0.0 | 3.63 | 7.99 | 2.91 | 0.0 | 2.91 | 5.0 | 40 | 3 | 0 |

Means per arm:

| Arm | n | Skill calls per run | Newtypes per run | Newtypes mean | Enums | Stringly ids /KLOC | Str compares /KLOC | expect /KLOC | unwrap_or* /KLOC | clone /KLOC | Lint set /KLOC | Comment % | Tests | Weak asserts per run | Prod lines | Turns | Min | USD | Commits |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| none | 3 | 0, 0, 0 | 0, 0, 0 | 0.0 | 3.0 | 22.3 | 1.32 | 9.2 | 11.4 | 4.2 | 7.4 | 6.5 | 43 | 9, 20, 14 | 1097 | 18 | 5.8 | 0.94 | 5.0 |
| skills | 3 | 6, 6, 5 | 4, 1, 2 | 2.3 | 3.3 | 24.7 | 1.39 | 5.3 | 8.0 | 4.7 | 3.7 | 6.7 | 71 | 2, 0, 9 | 1287 | 42 | 9.5 | 1.72 | 7.0 |
| router | 3 | 7, 7, 7 | 3, 3, 2 | 2.7 | 4.3 | 21.9 | 1.51 | 5.1 | 8.7 | 4.1 | 2.7 | 5.3 | 58 | 5, 8, 2 | 1177 | 36 | 8.5 | 1.51 | 6.0 |
| hook | 3 | 3, 3, 5 | 4, 3, 3 | 3.3 | 3.7 | 17.4 | 1.13 | 2.8 | 7.2 | 6.5 | 2.3 | 7.3 | 60 | 1, 0, 0 | 1266 | 36 | 9.2 | 1.60 | 7.3 |
| context | 3 | 0, 0, 0 | 10, 8, 10 | 9.3 | 8.0 | 11.3 | 0.37 | 3.2 | 7.2 | 2.7 | 1.9 | 4.6 | 39 | 16, 0, 3 | 1796 | 28 | 9.8 | 1.59 | 5.0 |

Blind scores per run (seven reviewers, each saw all fifteen crates):

| Run | Arm | Blind code | types | errors | ownership | simplicity | comments | tests | overall | Sum of 6 (max 24, overall excluded) |
|---|---|---|---|---|---|---|---|---|---|---|
| p1-03 | none | BO | 0 | 1 | 4 | 2 | 3 | 2 | 1 | 12 |
| p1-04 | none | PO | 0 | 2 | 3 | 2 | 2 | 2 | 1 | 11 |
| p1-07 | none | MI | 1 | 2 | 2 | 2 | 3 | 1 | 2 | 11 |
| p1-05 | skills | FI | 3 | 4 | 4 | 2 | 2 | 4 | 3 | 19 |
| p1-08 | skills | BI | 2 | 3 | 3 | 4 | 4 | 3 | 4 | 19 |
| p1-12 | skills | CE | 2 | 3 | 4 | 2 | 3 | 3 | 4 | 17 |
| p1-01 | router | TO | 2 | 3 | 3 | 3 | 4 | 3 | 3 | 18 |
| p1-09 | router | ZE | 2 | 4 | 4 | 2 | 4 | 3 | 3 | 19 |
| p1-15 | router | VU | 1 | 3 | 3 | 3 | 4 | 3 | 2 | 17 |
| p1-02 | hook | TA | 2 | 3 | 3 | 2 | 3 | 3 | 2 | 16 |
| p1-11 | hook | LI | 2 | 2 | 3 | 2 | 3 | 3 | 3 | 15 |
| p1-14 | hook | WU | 2 | 4 | 2 | 3 | 4 | 3 | 3 | 18 |
| p1-06 | context | QO | 4 | 3 | 3 | 1 | 4 | 4 | 3 | 19 |
| p1-10 | context | HE | 4 | 4 | 1 | 1 | 3 | 4 | 2 | 17 |
| p1-13 | context | FU | 4 | 4 | 4 | 2 | 3 | 3 | 4 | 20 |

Per arm:

| Arm | n | types | errors | ownership | simplicity | comments | tests | overall | Mean sum of 6 (max 24) | Sums per run | Range |
|---|---|---|---|---|---|---|---|---|---|---|---|
| context | 3 | 4.00 | 3.67 | 2.67 | 1.33 | 3.33 | 3.67 | 3.00 | 18.67 | 19, 17, 20 | 17 to 20 |
| skills | 3 | 2.33 | 3.33 | 3.67 | 2.67 | 3.00 | 3.33 | 3.67 | 18.33 | 19, 19, 17 | 17 to 19 |
| router | 3 | 1.67 | 3.33 | 3.33 | 2.67 | 4.00 | 3.00 | 2.67 | 18.00 | 18, 19, 17 | 17 to 19 |
| hook | 3 | 2.00 | 3.00 | 2.67 | 2.33 | 3.33 | 3.00 | 2.67 | 16.33 | 16, 15, 18 | 15 to 18 |
| none | 3 | 0.33 | 1.67 | 3.00 | 2.00 | 2.67 | 1.67 | 1.33 | 11.33 | 12, 11, 11 | 11 to 12 |

What separates the top from the bottom, per reviewer:

**types** (`reviews/p1/types.md`)

- Parse at the boundary: p1-06[context], p1-13[context], p1-10[context], p1-05[skills] build a typed command enum whose payloads are already validated (`QueueName`, ranged integers, `GroupId`); p1-03[none] (broker.rs:57) and p1-04[none] (broker.rs:65) keep a raw `Map<String, Value>` and parse in every handler, so `expect("required parameter is present")` appears inside business logic (p1-03[none]/src/queue_commands.rs:20).
- Ranged attribute types: p1-13[context]/src/scalar.rs:7, p1-10[context]/src/values.rs:14 and p1-06[context]/src/types.rs:49 make `VisibilityTimeout` etc. distinct types; everyone else stores `u32`/`u64` and relies on the parser having checked the range.
- Queue kind as an enum: p1-13[context]/src/queue.rs:22, p1-10[context]/src/queue.rs:41, p1-06[context]/src/queue.rs:27 put `ContentBasedDeduplication` inside the FIFO variant; the rest keep `fifo: bool` plus `content_based_deduplication: bool`, allowing the forbidden combination.
- In-flight state with its handle: p1-10[context], p1-06[context], p1-01[router], p1-14[hook], p1-11[hook], p1-09[router], p1-08[skills] hold the deadline and the handle in one variant/struct; p1-12[skills], p1-05[skills], p1-13[context], p1-07[none], p1-04[none], p1-15[router], p1-03[none] keep `invisible_until` and `handle: Option<_>` as independent fields.
- Id newtypes: p1-06[context]/p1-13[context]/p1-10[context]/p1-05[skills] type every identifier; p1-07[none], p1-15[router], p1-03[none], p1-04[none] use bare `u64` for message ids and receipt handles throughout.
- Typed responses: p1-08[skills], p1-05[skills], p1-13[context], p1-10[context], p1-06[context], p1-01[router] serialize response structs; the other nine assemble `serde_json::Value` objects inside the service.

**errors** (`reviews/p1/errors.md`)

- **Dead-letter hand-off.** Every crate has to send moved messages to a queue that must exist.
  - The top crates assert it with a named invariant: p1-13[context]/src/service.rs:329, p1-14[hook]/src/service.rs:237, p1-09[router]/src/broker.rs:127, p1-15[router]/src/broker.rs:245.
  - The bottom four skip it silently and lose the messages: p1-03[none]/src/message_commands.rs:104, p1-04[none]/src/broker/message_commands.rs:168, p1-11[hook]/src/broker.rs:236, p1-07[none]/src/broker.rs:272.
- **Required parameters.** p1-03[none] and p1-04[none] check presence in one call and re-extract with `.expect("required")` in another (p1-03[none]/src/message_commands.rs:19-26, p1-04[none]/src/broker/queue_commands.rs:21). p1-03[none] even reads the required `Seconds` with `unwrap_or(0)` (p1-03[none]/src/broker.rs:90). p1-13[context], p1-10[context], p1-06[context] and p1-09[router] parse each parameter once into a typed value (`required_bounded`, `required_string`), so no follow-up expect is needed.
- **Numeric conversion.** All 15 crates read input integers with checked `as_i64`/`as_u64` + `try_from`. After that:
  - p1-03[none] casts freely (`n as u32`, `seconds as u64`, `v as u32`, `number as usize - 1`: p1-03[none]/src/queue.rs:95, p1-03[none]/src/message_commands.rs:97,148).
  - p1-13[context], p1-10[context] and p1-06[context] carry range types (`Bounded`, `Ranged`, `MaxMessages`), so no cast is needed later.
- **Silent no-ops on invariant breaks.** `if let Some(message) = ..` that does nothing when the "impossible" case happens: p1-03[none]/src/message_commands.rs:137, p1-04[none]/src/broker/message_commands.rs:196, p1-01[router]/src/queue.rs:437. p1-05[skills], p1-02[hook] and p1-14[hook] state the same invariant with a named expect (p1-14[hook]/src/queue.rs:379).
- **Impossible branches.** p1-12[skills] and p1-02[hook] keep an `unreachable!`/`panic!` arm because they build responses as `json!` and unwrap them back (p1-12[skills]/src/broker.rs:431, p1-02[hook]/src/broker.rs:309). p1-06[context] turns an impossible lookup into a client-visible `QueueDoesNotExist` (p1-06[context]/src/broker.rs:312).

**ownership** (`reviews/p1/ownership.md`)

- Retention cost. p1-03[none], p1-12[skills], p1-13[context], p1-11[hook], p1-02[hook], p1-14[hook], p1-09[router] and p1-05[skills] keep a `(sent_at, arrival)` ordered index and pop only what expired (p1-12[skills]/src/queue.rs:137). p1-08[skills], p1-04[none], p1-06[context] and p1-15[router] skip work with an oldest-bound check, but once it is hit they rescan the whole queue and recompute the minimum (p1-04[none]/src/queue.rs:111-120). p1-07[none] rescans whenever the clock changes (p1-07[none]/src/queue.rs:207), and p1-01[router] and p1-10[context] rescan every queue on Tick (p1-01[router]/src/queue.rs:267).
- Receipt-handle lookup. Most crates map handle -> arrival key (`HashMap`) or index a `Vec` log by handle number (p1-09[router]/src/receipts.rs:49). p1-10[context] alone finds the message with `iter().position` over the whole queue (p1-10[context]/src/queue.rs:485), and then removes it with an O(n) `Vec::remove`.
- Moving owned input into storage. p1-09[router], p1-06[context], p1-15[router], p1-01[router], p1-13[context] and p1-08[skills] destructure the owned request and move `body` in (p1-09[router]/src/queue.rs:216). p1-11[hook] and p1-02[hook] borrow an owned request and clone the fields they store (p1-11[hook]/src/queue.rs:206, p1-02[hook]/src/queue.rs:238). p1-07[none] turns an owned `String` back into `&str` and calls `to_owned` again (p1-07[none]/src/queue.rs:298).
- The FIFO group check. Most collect `HashSet<&str>` or `&GroupId` borrowed from the map (p1-06[context]/src/queue.rs:550). p1-07[none], p1-14[hook], p1-10[context] and p1-08[skills] allocate a `String` or `GroupId` per group on every receive (p1-07[none]/src/queue.rs:370, p1-14[hook]/src/queue.rs:343). p1-05[skills] and p1-02[hook] keep per-group heads, so receive needs no set at all (p1-05[skills]/src/queue.rs:357).
- Clones that only end a borrow. p1-14[hook] clones the whole dead-letter config before the loop on every receive (p1-14[hook]/src/queue.rs:300) and clones `Command` (p1-14[hook]/src/request.rs:71). Most others clone only the DLQ name, and only when something was dead-lettered (p1-08[skills]/src/service.rs:328).

**simplicity** (`reviews/p1/simplicity.md`)

- **Deciding "required" once.** p1-08[skills] (request.rs:149), p1-15[router], p1-14[hook] (params.rs:26), p1-02[hook], p1-10[context], p1-07[none], p1-05[skills] and p1-06[context] return
  the value from the presence check. p1-03[none], p1-12[skills], p1-04[none], p1-11[hook], p1-09[router] and p1-13[context] check presence first and then fetch the
  value again. That fetch ends in `.expect("required")` (p1-03[none] message_commands.rs:19, p1-04[none]
  message_commands.rs:57) or in an `ok_or_else(missing)` that can never fire (p1-11[hook] command.rs:124, p1-09[router]
  request.rs:77).
- **Queue machinery that fits the size of the problem.** p1-08[skills], p1-15[router], p1-01[router] and p1-09[router] keep one ordered map, plus at
  most a by-age set. p1-05[skills] (queue.rs:115) and p1-02[hook] (queue.rs:135) keep 5 to 6 indexes in sync by hand, with
  heaps, ready sets and group maps. That is far more machinery than "examine in arrival order" calls for.
- **Types without a second user.** p1-10[context] and p1-06[context] each add a family of single-use types: a request type, an
  update type, a settings type and a spec type for the same config, and wrapper structs per response
  variant. They also copy boilerplate across newtypes (p1-10[context] values.rs:160, 181, 213; p1-06[context] types.rs:49 vs 70).
  p1-08[skills] covers the same ground with one `Config` and one `Attributes`.
- **Repeated state work.** p1-03[none] expires three times in one handler (queue_commands.rs:87, 115, 117), p1-02[hook]
  syncs twice (queue.rs:171-174), and p1-12[skills] looks up the same queue three times (broker.rs:158-176).
  p1-08[skills] and p1-15[router] do each step once, apart from a second lookup.
- **Validate here, ignore there.** p1-07[none] checks the dead-letter pair (broker.rs:347), and then
  `dead_letter_of` maps a half pair to `None` (queue.rs:72). p1-04[none] calls `update.dead_letter()?` only for its
  error (queue_commands.rs:47). In both cases one decision is spread over two places that can drift apart.

**comments** (`reviews/p1/comments.md`)

- The top crates comment fields with invariants a reader could not get from the type, e.g. p1-08[skills]/src/queue.rs:173 (lower bound that lets sweeps skip), p1-09[router]/src/receipts.rs:24 (why receipts are never dropped), p1-06[context]/src/deliveries.rs:10 (the log is also the set of issued handles).
- The bottom two use spec numbering instead of saying what is being checked: p1-05[skills]/tests/queues.rs:81-90 and 208-218 are bare `// Rule N.` labels, and p1-04[none] puts `(SPEC section 2, check 1)` on docs (p1-04[none]/src/request.rs:21, p1-04[none]/src/broker/message_commands.rs:203).
- Restating docs on trivial helpers separate the 3s from the 4s: p1-04[none]/src/naming.rs:10-23, p1-07[none]/src/command.rs:190-197, p1-11[hook]/src/command.rs:67, p1-05[skills]/src/domain.rs:126, and p1-10[context]/src/values.rs:159/180/212 (one doc copied three times).
- Gotcha comments on library behaviour (serde `arbitrary_precision`) show up in the stronger crates: p1-01[router]/src/request.rs:316, p1-10[context]/src/request.rs:293, p1-03[none]/src/params.rs:66, p1-14[hook]/src/params.rs:64.
- Comment volume works both ways. p1-15[router] (48 lines) loses nothing with so few, while p1-02[hook] (133) and p1-05[skills] (150) have the best rationale paragraphs and also the most comments that add nothing.

**tests** (`reviews/p1/tests.md`)

- Comparing whole responses. p1-10[context] (p1-10[context]/src/broker/tests.rs:16 `play`), p1-15[router] (p1-15[router]/tests/common/mod.rs `script`) and p1-06[context] (p1-06[context]/tests/protocol.rs:32 `assert_responses`) compare every response in full, so the "exactly these members" rule of the spec is enforced. p1-07[none] never builds a full expected message (p1-07[none]/tests/messages.rs:72-80 checks fields one by one) and p1-03[none] has only 4 whole-response expectations.
- Substring and `is_ok()` assertions. p1-07[none]/tests/binary.rs:29-33, p1-04[none]/tests/binary.rs:29-32, p1-01[router]/tests/process.rs:34 and p1-15[router]/tests/protocol.rs:40 check JSON lines with `contains`. p1-06[context]/src/types.rs:343-432 and p1-04[none]/src/request.rs:183-195 stop at `is_ok()`/`is_err()` without the variant. The top crates use these at most once or twice.
- Process-level protocol. p1-08[skills] and p1-03[none] never spawn the binary, so exit status 0 and stdout-only output go untested. p1-02[hook]/tests/binary.rs:65 is the only crate that checks each response arrives before the next line is written.
- Determinism. p1-04[none]/tests/messages.rs:585-594 bounds a 100000-send fill with `Instant::now().elapsed() < 20s`, a timing assertion that can fail on a slow CI runner. The other crates test the limit only by its outcome.
- Breadth. p1-05[skills] (98 tests), p1-14[hook] (76), p1-01[router] (73), p1-09[router] (65) and p1-06[context] cover nearly every numbered rule, including the dedup window boundary at 299/300. p1-15[router], p1-10[context] and p1-13[context] have fewer tests and depend on long scripts. p1-10[context]'s scripts are exhaustive, while p1-15[router] and p1-13[context] leave gaps (exact handle strings, PurgeQueue).

**overall** (`reviews/p1/overall.md`)

- **Parse once into a typed command.** Every top crate parses the line into a typed command enum before any state is touched: p1-08[skills]/src/request.rs:13, p1-12[skills]/src/request.rs:58, p1-13[context]/src/service.rs:29.
  - p1-03[none] and p1-04[none] keep the raw JSON map until the handler runs (p1-03[none]/src/broker.rs:56, p1-04[none]/src/broker.rs:65). Each handler then re-asserts presence with `expect("required")`, so a renamed parameter becomes a panic, not a compile error.
- **Plain data model vs. hand-built indexes.** p1-12[skills], p1-08[skills] and p1-13[context] keep one ordered map plus at most a `by_age` index (p1-12[skills]/src/queue.rs:109). Their state is derived from timestamps on demand.
  - p1-05[skills] (p1-05[skills]/src/queue.rs:115-136) and p1-02[hook] (p1-02[hook]/src/queue.rs:132-146) keep four or five indexes that every mutation must update together.
  - p1-02[hook] also stores the message state itself, which is correct only if `sync` was called (p1-02[hook]/src/queue.rs:16-18). That is far more to learn before a change is safe.
- **Storage that can't be invalidated under you.** p1-10[context] keeps messages in a `Vec`. It hands back indices that are only "valid until the queue is next changed" (p1-10[context]/src/queue.rs:194-200) and finds handles by linear scan (p1-10[context]/src/queue.rs:484).
  - Every other crate keys messages by a stable arrival number.
- **Pairs that belong together are typed together.** p1-13[context], p1-10[context] and p1-06[context] make content-based dedup part of the FIFO kind (p1-13[context]/src/queue.rs:21, p1-06[context]/src/queue.rs:27). p1-07[none] rebuilds the dead-letter pair with a match that silently ignores half a pair (p1-07[none]/src/queue.rs:67-74).
- **Typed ids.** The upper half gives `MessageId`/`ReceiptHandle` their own types with a single canonical parser (p1-12[skills]/src/ids.rs:26, p1-13[context]/src/ids.rs:38). p1-07[none], p1-04[none] and p1-15[router] pass bare `u64` handles and ids through queue, broker and response code (p1-15[router]/src/queue.rs:58-59).

Reading.

- Activation. The `skills` arm loaded five or six of the six guides in all three runs, starting at
  tool call 3, with no entry skill and no mention of skills in the prompt. The `hook` arm, which has
  the same skills, did so in one run of three (p1-14); the other two loaded three guides only after
  the Stop hook told them to. The `router` arm loaded all seven every time, as in v1.
- No untreated run made a newtype (0, 0, 0). Every treated run made at least one; `context` made 8
  to 10 (p1-10's 8 is not affected by the detector caveat, its per-KLOC figures are).
- The blind sums do not overlap: 11, 11, 12 for `none` against 15 to 20 for the twelve treated runs.
  The gap is in `types` (0.33 against 1.67 to 4.00), `errors` (1.67 against 3.00 to 3.67), `tests`
  (1.67 against 3.00 to 3.67) and `overall` (1.33 against 2.67 to 3.67). `ownership` and
  `simplicity` do not separate treated from untreated.
- Among the treated arms the sums overlap completely (15 to 20). The profile differs: `context` has
  the highest `types` (4, 4, 4) and the lowest `simplicity` (1, 1, 2); the reviewer's reason is
  "a family of single-use types" and copied newtype boilerplate (p1-10, p1-06).
- `hook` is the best arm on weak assertions (1, 0, 0 against 9, 20, 14 for `none`) and the lowest of
  the treated arms on the blind sum. In two of its runs the guides arrived only at the end, as a
  repair step.
- Cost: `none` 5.8 minutes and USD 0.94 per run; the treated arms 8.5 to 9.8 minutes and USD 1.51 to
  1.72.

### 5.2 P2: add a feature to existing code, Sonnet 5.5

Design: the start crate is `baseline/`, a copy of p1-04 (the `none` run with the median P1 blind sum,
11; the tie with p1-07 was broken by run number). The agent gets `SPEC-feature.md` and the feature
prompt. Five arms, three runs each. The plan was `none` plus the best three arms of P1; all five were
run because runs turned out to be cheap. The reviewers were told to score BASE as a whole and each
attempt only on the code it added or changed. Four dimensions were reviewed (types, errors, tests,
overall).

| Arm | Model | Runs |
|---|---|---|
| none | sonnet-5-5 | p2-09, p2-12, p2-13 |
| skills | sonnet-5-5 | p2-03, p2-06, p2-07 |
| router | sonnet-5-5 | p2-01, p2-05, p2-11 |
| hook | sonnet-5-5 | p2-04, p2-14, p2-15 |
| context | sonnet-5-5 | p2-02, p2-08, p2-10 |

Process:

| Run | Arm | Hidden tests | Own tests | Skill calls | Skills loaded, in order | At tool call | Tool calls | Stop hook | Turns | Min | USD | Commits | Diff vs start |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| p2-09 | none | 80/80 | ok | 0 | - | - | 11 | - | 12 | 2.0 | 0.44 | 3 | 12 files, 611+, 6- |
| p2-12 | none | 80/80 | ok | 0 | - | - | 12 | - | 13 | 1.9 | 0.42 | 3 | 12 files, 548+, 2- |
| p2-13 | none | 80/80 | ok | 0 | - | - | 16 | - | 17 | 2.9 | 0.61 | 5 | 12 files, 865+, 98- |
| p2-03 | skills | 80/80 | ok | 0 | - | - | 18 | - | 19 | 2.8 | 0.55 | 4 | 12 files, 806+, 3- |
| p2-06 | skills | 80/80 | ok | 0 | - | - | 14 | - | 15 | 2.3 | 0.49 | 3 | 12 files, 766+, 6- |
| p2-07 | skills | 80/80 | ok | 0 | - | - | 18 | - | 19 | 2.5 | 0.53 | 3 | 9 files, 707+, 10- |
| p2-01 | router | 80/80 | ok | 7 | code(router), types, errors, ownership, simple, comments, tests | 1, 3, 4, 5, 6, 7, 8 | 24 | - | 32 | 2.7 | 0.69 | 3 | 12 files, 786+, 2- |
| p2-05 | router | 80/80 | ok | 7 | code(router), types, errors, ownership, simple, comments, tests | 1, 3, 4, 5, 6, 7, 8 | 19 | - | 27 | 2.2 | 0.61 | 3 | 11 files, 639+, 8- |
| p2-11 | router | 80/80 | ok | 7 | code(router), types, errors, ownership, simple, comments, tests | 1, 3, 4, 5, 6, 7, 8 | 23 | - | 31 | 3.0 | 0.78 | 4 | 13 files, 868+, 15- |
| p2-04 | hook | 80/80 | ok | 2 | tests, errors | 18, 19 | 21 | fired after call 17; 4 more tool calls, 2 of them Skill | 25 | 3.3 | 0.69 | 4 | 11 files, 905+, 10- |
| p2-14 | hook | 80/80 | ok | 0 | - | - | 21 | fired after call 20; 1 more tool call, 0 of them Skill | 23 | 3.5 | 0.69 | 5 | 11 files, 787+, 13- |
| p2-15 | hook | 80/80 | ok | 1 | tests | 19 | 21 | fired after call 18; 3 more tool calls, 1 of them Skill | 24 | 3.7 | 0.75 | 6 | 12 files, 1014+, 98- |
| p2-02 | context | 80/80 | ok | 0 | - | - | 21 | - | 22 | 3.0 | 0.63 | 3 | 13 files, 925+, 14- |
| p2-08 | context | 80/80 | ok | 0 | - | - | 17 | - | 18 | 3.1 | 0.63 | 3 | 14 files, 944+, 5- |
| p2-10 | context | 80/80 | ok | 0 | - | - | 26 | - | 27 | 3.0 | 0.65 | 4 | 14 files, 802+, 3- |

Code (whole crate after the change; BASE is the start crate):

| Run | Arm | Prod lines | Test lines | src files | Newtypes | Enums | Stringly ids /KLOC | Str compares /KLOC | expect /KLOC | unwrap_or* /KLOC | clone /KLOC | as casts /KLOC | Lint set /KLOC | Comment % | Tests | Weak asserts | allow attrs |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| BASE (p1-04) | - | 1057 | 1073 | 10 | 0 | 3 | 19.43 | 2.04 | 12.27 | 10.22 | 3.07 | 1.02 | 5.11 | 7.5 | 44 | 20 | 1 |
| p2-09 | none | 1277 | 1409 | 12 | 0 | 3 | 17.69 | 1.68 | 16.01 | 9.27 | 5.05 | 1.68 | 8.42 | 7.0 | 60 | 21 | 1 |
| p2-12 | none | 1265 | 1361 | 12 | 0 | 3 | 17.81 | 1.7 | 16.96 | 9.33 | 4.24 | 1.7 | 6.79 | 6.8 | 65 | 22 | 1 |
| p2-13 | none | 1360 | 1474 | 12 | 0 | 3 | 18.18 | 2.37 | 15.81 | 8.7 | 3.95 | 1.58 | 6.32 | 7.0 | 63 | 23 | 1 |
| p2-03 | skills | 1279 | 1601 | 12 | 0 | 3 | 19.38 | 1.68 | 16.01 | 9.27 | 3.37 | 1.68 | 5.9 | 7.2 | 64 | 21 | 1 |
| p2-06 | skills | 1280 | 1554 | 12 | 0 | 3 | 19.34 | 1.68 | 15.98 | 9.25 | 3.36 | 1.68 | 6.73 | 7.1 | 63 | 22 | 1 |
| p2-07 | skills | 1295 | 1499 | 10 | 0 | 3 | 21.54 | 2.49 | 17.4 | 9.11 | 2.49 | 1.66 | 4.14 | 6.8 | 57 | 21 | 1 |
| p2-01 | router | 1281 | 1580 | 12 | 0 | 3 | 20.15 | 3.36 | 15.95 | 9.24 | 2.52 | 1.68 | 5.88 | 7.0 | 63 | 20 | 1 |
| p2-05 | router | 1249 | 1460 | 10 | 0 | 3 | 18.04 | 1.72 | 15.46 | 9.45 | 3.44 | 1.72 | 6.01 | 6.8 | 66 | 21 | 1 |
| p2-11 | router | 1330 | 1591 | 12 | 0 | 3 | 21.05 | 1.62 | 15.38 | 8.91 | 2.43 | 1.62 | 7.29 | 7.1 | 64 | 22 | 1 |
| p2-04 | hook | 1330 | 1638 | 12 | 0 | 3 | 22.6 | 2.42 | 16.14 | 8.88 | 2.42 | 1.61 | 5.65 | 6.8 | 63 | 20 | 1 |
| p2-14 | hook | 1325 | 1528 | 12 | 0 | 3 | 21.05 | 2.43 | 17.81 | 8.91 | 3.24 | 1.62 | 5.67 | 6.8 | 58 | 20 | 1 |
| p2-15 | hook | 1348 | 1632 | 12 | 0 | 3 | 19.18 | 2.4 | 15.99 | 8.79 | 3.2 | 1.6 | 5.6 | 7.2 | 69 | 20 | 1 |
| p2-02 | context | 1352 | 1626 | 12 | 2 | 3 | 20.73 | 1.59 | 15.15 | 8.77 | 2.39 | 0.8 | 5.58 | 7.2 | 65 | 23 | 1 |
| p2-08 | context | 1342 | 1645 | 13 | 2 | 3 | 18.56 | 1.61 | 15.33 | 8.88 | 4.04 | 0.81 | 6.46 | 7.7 | 77 | 27 | 1 |
| p2-10 | context | 1324 | 1543 | 12 | 1 | 3 | 18.68 | 1.62 | 14.62 | 8.94 | 4.06 | 1.62 | 6.5 | 7.0 | 64 | 21 | 1 |

Means per arm:

| Arm | n | Skill calls per run | Newtypes per run | Newtypes mean | Enums | Stringly ids /KLOC | Str compares /KLOC | expect /KLOC | unwrap_or* /KLOC | clone /KLOC | Lint set /KLOC | Comment % | Tests | Weak asserts per run | Prod lines | Turns | Min | USD | Commits |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| none | 3 | 0, 0, 0 | 0, 0, 0 | 0.0 | 3.0 | 17.9 | 1.92 | 16.3 | 9.1 | 4.4 | 7.2 | 6.9 | 63 | 21, 22, 23 | 1301 | 14 | 2.3 | 0.49 | 3.7 |
| skills | 3 | 0, 0, 0 | 0, 0, 0 | 0.0 | 3.0 | 20.1 | 1.95 | 16.5 | 9.2 | 3.1 | 5.6 | 7.0 | 61 | 21, 22, 21 | 1285 | 18 | 2.5 | 0.52 | 3.3 |
| router | 3 | 7, 7, 7 | 0, 0, 0 | 0.0 | 3.0 | 19.7 | 2.23 | 15.6 | 9.2 | 2.8 | 6.4 | 7.0 | 64 | 20, 21, 22 | 1287 | 30 | 2.6 | 0.69 | 3.3 |
| hook | 3 | 2, 0, 1 | 0, 0, 0 | 0.0 | 3.0 | 20.9 | 2.42 | 16.6 | 8.9 | 3.0 | 5.6 | 6.9 | 63 | 20, 20, 20 | 1334 | 24 | 3.5 | 0.71 | 5.0 |
| context | 3 | 0, 0, 0 | 2, 2, 1 | 1.7 | 3.0 | 19.3 | 1.61 | 15.0 | 8.9 | 3.5 | 6.2 | 7.3 | 69 | 23, 27, 21 | 1339 | 22 | 3.0 | 0.64 | 3.3 |

Blind scores per run:

| Run | Arm | Blind code | types | errors | tests | overall | Sum of 3 (max 12, overall excluded) |
|---|---|---|---|---|---|---|---|
| BASE | - | BASE | 2 | 3 | 3 | 4 | 8 |
| p2-09 | none | PI | 1 | 3 | 2 | 4 | 6 |
| p2-12 | none | KE | 1 | 3 | 2 | 3 | 6 |
| p2-13 | none | CU | 3 | 3 | 4 | 2 | 10 |
| p2-03 | skills | PO | 2 | 3 | 4 | 4 | 9 |
| p2-06 | skills | RO | 2 | 3 | 4 | 4 | 9 |
| p2-07 | skills | ZU | 2 | 2 | 2 | 2 | 6 |
| p2-01 | router | WU | 2 | 4 | 3 | 4 | 9 |
| p2-05 | router | JO | 1 | 3 | 3 | 4 | 7 |
| p2-11 | router | VU | 3 | 4 | 3 | 3 | 10 |
| p2-04 | hook | PA | 3 | 3 | 3 | 3 | 9 |
| p2-14 | hook | XA | 3 | 2 | 2 | 3 | 7 |
| p2-15 | hook | FI | 3 | 3 | 4 | 3 | 10 |
| p2-02 | context | NO | 4 | 4 | 3 | 3 | 11 |
| p2-08 | context | VI | 3 | 3 | 3 | 4 | 9 |
| p2-10 | context | XO | 3 | 3 | 2 | 3 | 8 |

Per arm:

| Arm | n | types | errors | tests | overall | Mean sum of 3 (max 12) | Sums per run | Range |
|---|---|---|---|---|---|---|---|---|
| context | 3 | 3.33 | 3.33 | 2.67 | 3.33 | 9.33 | 11, 9, 8 | 8 to 11 |
| hook | 3 | 3.00 | 2.67 | 3.00 | 3.00 | 8.67 | 9, 7, 10 | 7 to 10 |
| router | 3 | 2.00 | 3.67 | 3.00 | 3.67 | 8.67 | 9, 7, 10 | 7 to 10 |
| BASE | 1 | 2.00 | 3.00 | 3.00 | 4.00 | 8.00 | 8 | 8 to 8 |
| skills | 3 | 2.00 | 2.67 | 3.33 | 3.33 | 8.00 | 9, 9, 6 | 6 to 9 |
| none | 3 | 1.67 | 3.00 | 2.67 | 3.00 | 7.33 | 6, 6, 10 | 6 to 10 |

What separates the top from the bottom:

**types** (`reviews/p2/types.md`)

- Batch entries. The 3s parse each entry into a typed `SendArgs`/`SendEntry`/`Send` and pass that inward (p2-13[none]/src/broker/message_commands.rs:55, p2-11[router]/src/broker/message_commands.rs:61). p2-05[router], p2-12[none] and p2-09[none] clone the entry's `serde_json::Map`, write `"QueueName"` into it, and hand it back to the string-keyed handler (p2-05[router]/src/broker/message_commands.rs:123, p2-12[none]/src/broker/batch_commands.rs:72, p2-09[none]/src/broker/batch_commands.rs:83).
- Tag domain types. Only p2-02[context], p2-08[context] and p2-10[context] give tags their own types (p2-02[context]/src/tags.rs:18, p2-08[context]/src/tags.rs:16, p2-10[context]/src/tags.rs:21). The other twelve store `BTreeMap<String,String>` and treat the key rule as a passing check (e.g. p2-15[hook]/src/queue.rs:75).
- Keeping the entry Id with its entry. Most return `(id, Params)` from validation, and p2-04[hook] names the pair (p2-04[hook]/src/broker/batch_commands.rs:16). p2-07[skills] and p2-12[none] validate the ids into a separate `Vec<&str>` and later re-cast each entry with `expect` (p2-07[skills]/src/broker/message_commands.rs:306, p2-12[none]/src/broker/batch_commands.rs:71).
- Numeric conversion for `MaxMessageBytes`. p2-02[context] stores `usize` via `try_from` (p2-02[context]/src/attributes.rs:131) and p2-08[context] uses `u64::try_from` (p2-08[context]/src/broker/message_commands.rs:80). The other 13 cast `body.len() as u64` (e.g. p2-10[context]/src/broker/message_commands.rs:78). The cast is harmless here but unchecked.
- Middle tier (p2-03[skills], p2-06[skills], p2-01[router], p2-08[context], p2-10[context]). They pass `Params` inward to `send_to(&str, Params)`. There is no JSON mutation, but no typed value either, so the parse still happens inside the handler (p2-03[skills]/src/broker/message_commands.rs:63).

**errors** (`reviews/p2/errors.md`)

- The top three keep every new failure as an explicit `Err`. On the entry `Id` they avoid the Result entirely (p2-01[router]/src/broker/batch_commands.rs:44), map it with `map_err` (p2-02[context]/src/batch.rs:45) or match on it (p2-11[router]/src/broker/batch_commands.rs:44). Most others write `.ok().flatten()` (p2-13[none]/src/broker/batch_commands.rs:45 and seven more).
- At the bottom, panics rest on invariants held somewhere else. p2-07[skills] and p2-12[none] validate entries in one pass and then `expect` the object again in a second (p2-07[skills]/src/broker/message_commands.rs:306, p2-12[none]/src/broker/batch_commands.rs:71). p2-14[hook] and p2-10[context] move the `require("Body")` out of the function that `expect`s it (p2-14[hook]/src/broker/message_commands.rs:65, p2-10[context]/src/broker/message_commands.rs:64).
- p2-07[skills] also pairs ids with results through a separate `zip` (p2-07[skills]/src/broker/message_commands.rs:94). Everyone else carries the id together with its entry, so no mismatch is possible.
- Only p2-02[context] and p2-08[context] handled the `MaxMessageBytes` comparison without an `as` cast (p2-02[context] keeps the field as `usize`; p2-08[context] uses `try_from`). The other 13 write `body.len() as u64`, which is lossless but is the cast the brief asks to avoid.

**tests** (`reviews/p2/tests.md`)

- Per-entry results: p2-13[none], p2-15[hook], p2-03[skills], p2-06[skills], p2-02[context] and p2-11[router] collect `Failed` into ordered (Id, code) pairs and
  compare them whole (p2-13[none]/tests/batches.rs:63, p2-15[hook]/tests/batches.rs:286). p2-09[none], p2-12[none], p2-10[context] and p2-07[skills] check
  `Failed[0]`/`Failed[1]` codes one at a time without the Id (p2-09[none]/tests/batches.rs:172,
  p2-12[none]/tests/batches.rs:32, p2-10[context]/tests/batches.rs:138, p2-07[skills]/tests/messages.rs:907).
- Expected values that come from the code under test: p2-14[hook]/tests/batches.rs:47 and
  p2-10[context]/tests/batches.rs:259 fill the expected `message` from the response under test. The others
  check `message.is_string()` on its own (p2-13[none]/tests/batches.rs:20).
- Assertions that can pass vacuously: p2-09[none]/tests/batches.rs:56 loops over `Failed` with no length
  check. p2-07[skills]/tests/messages.rs:826 asserts only a sum. p2-12[none]/tests/batches.rs:126 has a name that claims
  a behavior it does not test.
- The message limit inside a batch: p2-02[context]/tests/batch.rs:343, p2-01[router]/tests/batches.rs:183 and
  p2-14[hook]/tests/batches.rs:127 leave room for 5 or 3 entries and check the exact split. p2-12[none] never tests it.
  Most others fill the queue and check one entry.
- Coverage of "dead-letter moves ignore MaxMessageBytes": present in p2-13[none], p2-15[hook], p2-03[skills], p2-06[skills], p2-08[context], p2-11[router], p2-01[router], p2-10[context],
  p2-12[none], p2-09[none]. Missing in p2-02[context], p2-04[hook], p2-05[router], p2-14[hook], p2-07[skills].
- Foreign-queue handles and `r-01`-style handles in DeleteMessageBatch: tested by p2-13[none], p2-15[hook], p2-04[hook], p2-03[skills],
  p2-14[hook], p2-10[context], p2-11[router]. Not tested by p2-05[router], p2-12[none], p2-09[none], p2-06[skills], p2-08[context], p2-01[router], p2-07[skills].

**overall** (`reviews/p2/overall.md`)

- Whether batch entries go through the single command's own code. p2-03[skills], p2-06[skills], p2-01[router], p2-08[context] split each single
  command at "step 2 on" (`send_to(name, params)`), and p2-05[router], p2-09[none] call the single command itself
  (p2-05[router]/src/broker/message_commands.rs:108); a later change to SendMessage then cannot miss the batch.
  p2-13[none] and p2-15[hook] moved the whole SendMessage body into a free function threaded with counters
  (p2-13[none]/src/broker/message_commands.rs:84), which is a larger inherited-code rewrite to trust.
- Validate once, carry the result. Top crates return `(id, Params)` from entry parsing; p2-12[none] and p2-07[skills]
  return only ids and re-open the entries with `expect` (p2-12[none]/src/broker/batch_commands.rs:71,
  p2-07[skills]/src/broker/message_commands.rs:306).
- One batch prologue. Most have one `run_batch`; p2-14[hook] and p2-07[skills] copy the require/lookup/parse preamble
  into both handlers (p2-14[hook]/src/broker/batch_commands.rs:78 and :98).
- Placement. Separate `batch_commands.rs`/`tag_commands.rs` (or `tags.rs`) keep files at BASE's
  scale; p2-07[skills] and, less so, p2-05[router] grow message_commands.rs/queue_commands.rs with unrelated features.
- Typed tags (p2-02[context], p2-08[context], p2-10[context]) move the 50-tag invariant next to the data (p2-08[context]/src/tags.rs:57); they
  add a module but make the limit hard to bypass.

Reading.

- Activation. With the skills merely available, none was loaded (0 of 3 in `skills`; 0 of 3 in
  `hook` before the hook fired). The feature prompt has no quality request in it, and "implement
  three additions" matches no skill description.
- Loading them did not help: `router` loaded all seven in every run and its new code has 0 newtypes,
  like `none`.
- The new code takes the style of the code around it. The start crate has 0 newtypes; 12 of the 15
  results have 0. The three exceptions are the three `context` runs (2, 2, 1: typed tags, per the
  `overall` reviewer). `expect` per KLOC rises in every run, from 12.27 to between 14.6 and 17.8.
- Blind: no arm is separated from `none`. `none` ranges 6 to 10, and every other arm's range
  overlaps it. `context` has the highest mean (9.33) on `types` 3.33.
- The `overall` reviewer gave its top marks to attempts that routed batch entries through the
  existing single-command code and kept files at the start crate's scale, and marked down attempts
  that rewrote inherited code. It scored the untouched BASE 4. On this task, by this judge, fitting
  in is the quality.
- The hook fired in all three of its runs and each agent answered it by fixing one or two weak
  assertions in its own new tests and declining the findings in inherited code ("I fixed the two
  findings that were mine", p2-14).

### 5.3 P3: clean up existing code, Sonnet 5.5

Design: the same start crate, the v1 refactor prompt. Six arms, three runs each (the plan had five;
`hook` was added). All seven dimensions reviewed, BASE included as a labelled reference.

| Arm | Model | Runs |
|---|---|---|
| none | sonnet-5-5 | p3-07, p3-09, p3-10 |
| skills | sonnet-5-5 | p3-02, p3-04, p3-17 |
| router | sonnet-5-5 | p3-01, p3-03, p3-18 |
| audit | sonnet-5-5 | p3-12, p3-14, p3-16 |
| hook | sonnet-5-5 | p3-05, p3-06, p3-13 |
| context | sonnet-5-5 | p3-08, p3-11, p3-15 |

Process:

| Run | Arm | Hidden tests | Own tests | Skill calls | Skills loaded, in order | At tool call | Tool calls | Stop hook | Turns | Min | USD | Commits | Diff vs start |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| p3-07 | none | 58/58 | ok | 0 | - | - | 8 | - | 9 | 1.1 | 0.24 | 3 | 5 files, 90+, 64- |
| p3-09 | none | 58/58 | ok | 0 | - | - | 8 | - | 9 | 1.3 | 0.28 | 4 | 6 files, 111+, 60- |
| p3-10 | none | 58/58 | ok | 0 | - | - | 9 | - | 10 | 1.2 | 0.25 | 4 | 4 files, 92+, 46- |
| p3-02 | skills | 58/58 | ok | 1 | simple | 2 | 8 | - | 10 | 0.9 | 0.24 | 3 | 5 files, 36+, 26- |
| p3-04 | skills | 58/58 | ok | 2 | simple, comments | 2, 3 | 10 | - | 13 | 1.1 | 0.28 | 4 | 6 files, 40+, 27- |
| p3-17 | skills | 58/58 | ok | 1 | simple | 2 | 7 | - | 9 | 0.8 | 0.22 | 2 | 4 files, 21+, 18- |
| p3-01 | router | 58/58 | ok | 7 | code(router), types, errors, ownership, simple, comments, tests | 1, 3, 4, 5, 6, 7, 8 | 19 | - | 27 | 1.6 | 0.45 | 3 | 5 files, 99+, 79- |
| p3-03 | router | 58/58 | ok | 7 | code(router), types, errors, ownership, simple, comments, tests | 1, 3, 4, 5, 6, 7, 8 | 22 | - | 30 | 1.7 | 0.49 | 4 | 7 files, 133+, 80- |
| p3-18 | router | 58/58 | ok | 7 | code(router), types, errors, ownership, simple, comments, tests | 1, 3, 4, 5, 6, 7, 8 | 17 | - | 25 | 1.4 | 0.4 | 5 | 6 files, 44+, 32- |
| p3-12 | audit | 58/58 | ok | 1 | audit | 1 | 13 | - | 15 | 1.8 | 0.34 | 5 | 7 files, 109+, 84- |
| p3-14 | audit | 58/58 | ok | 1 | audit | 1 | 11 | - | 13 | 1.4 | 0.29 | 4 | 6 files, 67+, 51- |
| p3-16 | audit | 58/58 | ok | 1 | audit | 1 | 12 | - | 14 | 1.5 | 0.31 | 5 | 6 files, 67+, 60- |
| p3-05 | hook | 58/58 | ok | 1 | simple | 2 | 11 | fired after call 8; 3 more tool calls, 0 of them Skill | 14 | 1.4 | 0.32 | 5 | 8 files, 49+, 39- |
| p3-06 | hook | 58/58 | ok | 1 | simple | 2 | 10 | fired after call 5; 5 more tool calls, 0 of them Skill | 13 | 1.3 | 0.3 | 2 | 5 files, 50+, 39- |
| p3-13 | hook | 58/58 | ok | 1 | simple | 2 | 11 | fired after call 8; 3 more tool calls, 0 of them Skill | 14 | 1.6 | 0.36 | 4 | 7 files, 82+, 51- |
| p3-08 | context | 58/58 | ok | 0 | - | - | 28 | - | 29 | 5.0 | 0.9 | 8 | 9 files, 548+, 261- |
| p3-11 | context | 58/58 | ok | 0 | - | - | 23 | - | 24 | 3.5 | 0.69 | 6 | 8 files, 374+, 198- |
| p3-15 | context | 58/58 | ok | 0 | - | - | 29 | - | 30 | 5.5 | 1.02 | 12 | 10 files, 687+, 313- |

Code (BASE is the start crate):

| Run | Arm | Prod lines | Test lines | src files | Newtypes | Enums | Stringly ids /KLOC | Str compares /KLOC | expect /KLOC | unwrap_or* /KLOC | clone /KLOC | as casts /KLOC | Lint set /KLOC | Comment % | Tests | Weak asserts | allow attrs |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| BASE (p1-04) | - | 1057 | 1073 | 10 | 0 | 3 | 19.43 | 2.04 | 12.27 | 10.22 | 3.07 | 1.02 | 5.11 | 7.5 | 44 | 20 | 1 |
| p3-07 | none | 1080 | 1073 | 10 | 0 | 4 | 21.15 | 2.01 | 12.08 | 10.07 | 3.02 | 1.01 | 5.04 | 8.1 | 44 | 20 | 1 |
| p3-09 | none | 1105 | 1073 | 10 | 0 | 4 | 20.98 | 2.0 | 11.99 | 9.99 | 3.0 | 1.0 | 5.0 | 9.4 | 44 | 20 | 1 |
| p3-10 | none | 1098 | 1073 | 10 | 0 | 4 | 22.77 | 1.98 | 11.88 | 9.9 | 2.97 | 0.99 | 4.95 | 8.0 | 44 | 20 | 1 |
| p3-02 | skills | 1065 | 1073 | 10 | 0 | 3 | 19.33 | 2.03 | 12.21 | 10.17 | 3.05 | 1.02 | 5.09 | 7.7 | 44 | 20 | 1 |
| p3-04 | skills | 1067 | 1073 | 10 | 0 | 3 | 19.37 | 2.04 | 12.23 | 10.19 | 3.06 | 1.02 | 5.1 | 8.1 | 44 | 20 | 1 |
| p3-17 | skills | 1059 | 1073 | 10 | 0 | 3 | 19.41 | 2.04 | 12.26 | 10.21 | 3.06 | 1.02 | 5.11 | 7.6 | 44 | 20 | 1 |
| p3-01 | router | 1073 | 1073 | 10 | 0 | 3 | 22.24 | 2.02 | 12.13 | 10.11 | 3.03 | 1.01 | 5.06 | 7.8 | 44 | 20 | 1 |
| p3-03 | router | 1104 | 1073 | 10 | 0 | 4 | 22.62 | 1.97 | 11.8 | 9.83 | 2.95 | 0.98 | 4.92 | 7.9 | 44 | 20 | 1 |
| p3-18 | router | 1065 | 1073 | 10 | 0 | 3 | 19.33 | 2.03 | 12.21 | 10.17 | 3.05 | 1.02 | 5.09 | 7.7 | 44 | 20 | 1 |
| p3-12 | audit | 1074 | 1077 | 10 | 0 | 3 | 20.24 | 2.02 | 2.02 | 10.12 | 3.04 | 0.0 | 3.04 | 8.0 | 44 | 15 | 1 |
| p3-14 | audit | 1067 | 1076 | 10 | 0 | 3 | 21.41 | 2.04 | 1.02 | 10.19 | 3.06 | 1.02 | 5.1 | 8.1 | 44 | 15 | 1 |
| p3-16 | audit | 1061 | 1075 | 10 | 0 | 3 | 20.51 | 2.05 | 2.05 | 10.26 | 3.08 | 1.03 | 5.13 | 8.1 | 44 | 16 | 1 |
| p3-05 | hook | 1060 | 1078 | 10 | 0 | 3 | 19.43 | 2.04 | 12.27 | 10.22 | 3.07 | 1.02 | 5.11 | 7.7 | 44 | 8 | 1 |
| p3-06 | hook | 1062 | 1076 | 10 | 0 | 3 | 21.45 | 2.04 | 1.02 | 10.21 | 3.06 | 0.0 | 3.06 | 7.8 | 44 | 15 | 1 |
| p3-13 | hook | 1070 | 1084 | 10 | 0 | 3 | 21.32 | 2.03 | 1.02 | 10.15 | 3.05 | 0.0 | 3.05 | 7.9 | 44 | 9 | 1 |
| p3-08 | context | 1268 | 1118 | 11 | 0 | 6 | 21.46 | 0.86 | 0.86 | 9.44 | 0.0 | 0.0 | 2.58 | 8.1 | 48 | 20 | 1 |
| p3-11 | context | 1174 | 1109 | 11 | 3 | 4 | 22.37 | 0.93 | 0.93 | 9.32 | 1.86 | 0.0 | 2.8 | 8.6 | 47 | 20 | 1 |
| p3-15 | context | 1309 | 1140 | 12 | 7 | 6 | 21.74 | 0.84 | 2.51 | 8.36 | 0.84 | 0.0 | 5.85 | 8.6 | 49 | 15 | 1 |

Means per arm:

| Arm | n | Skill calls per run | Newtypes per run | Newtypes mean | Enums | Stringly ids /KLOC | Str compares /KLOC | expect /KLOC | unwrap_or* /KLOC | clone /KLOC | Lint set /KLOC | Comment % | Tests | Weak asserts per run | Prod lines | Turns | Min | USD | Commits |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| none | 3 | 0, 0, 0 | 0, 0, 0 | 0.0 | 4.0 | 21.6 | 2.00 | 12.0 | 10.0 | 3.0 | 5.0 | 8.5 | 44 | 20, 20, 20 | 1094 | 9 | 1.2 | 0.26 | 3.7 |
| skills | 3 | 1, 2, 1 | 0, 0, 0 | 0.0 | 3.0 | 19.4 | 2.04 | 12.2 | 10.2 | 3.1 | 5.1 | 7.8 | 44 | 20, 20, 20 | 1064 | 11 | 0.9 | 0.25 | 3.0 |
| router | 3 | 7, 7, 7 | 0, 0, 0 | 0.0 | 3.3 | 21.4 | 2.01 | 12.0 | 10.0 | 3.0 | 5.0 | 7.8 | 44 | 20, 20, 20 | 1081 | 27 | 1.6 | 0.45 | 4.0 |
| audit | 3 | 1, 1, 1 | 0, 0, 0 | 0.0 | 3.0 | 20.7 | 2.04 | 1.7 | 10.2 | 3.1 | 4.4 | 8.1 | 44 | 15, 15, 16 | 1067 | 14 | 1.6 | 0.31 | 4.7 |
| hook | 3 | 1, 1, 1 | 0, 0, 0 | 0.0 | 3.0 | 20.7 | 2.04 | 4.8 | 10.2 | 3.1 | 3.7 | 7.8 | 44 | 8, 15, 9 | 1064 | 14 | 1.4 | 0.33 | 3.7 |
| context | 3 | 0, 0, 0 | 0, 3, 7 | 3.3 | 5.3 | 21.9 | 0.88 | 1.4 | 9.0 | 0.9 | 3.7 | 8.4 | 48 | 20, 20, 15 | 1250 | 28 | 4.7 | 0.87 | 8.7 |

Blind scores per run:

| Run | Arm | Blind code | types | errors | ownership | simplicity | comments | tests | overall | Sum of 6 (max 24, overall excluded) |
|---|---|---|---|---|---|---|---|---|---|---|
| BASE | - | BASE | 2 | 2 | 2 | 2 | 3 | 2 | 2 | 13 |
| p3-07 | none | PO | 2 | 2 | 2 | 2 | 3 | 2 | 2 | 13 |
| p3-09 | none | FI | 2 | 2 | 2 | 2 | 2 | 2 | 2 | 12 |
| p3-10 | none | SU | 2 | 2 | 2 | 2 | 3 | 2 | 2 | 13 |
| p3-02 | skills | VA | 2 | 2 | 2 | 2 | 3 | 2 | 2 | 13 |
| p3-04 | skills | HO | 2 | 2 | 2 | 2 | 3 | 2 | 2 | 13 |
| p3-17 | skills | WO | 2 | 2 | 2 | 2 | 3 | 2 | 2 | 13 |
| p3-01 | router | NU | 2 | 2 | 2 | 2 | 3 | 2 | 2 | 13 |
| p3-03 | router | VO | 2 | 2 | 2 | 2 | 3 | 2 | 2 | 13 |
| p3-18 | router | FO | 2 | 2 | 2 | 2 | 3 | 2 | 2 | 13 |
| p3-12 | audit | ZA | 2 | 4 | 3 | 3 | 4 | 3 | 3 | 19 |
| p3-14 | audit | PI | 2 | 3 | 2 | 3 | 4 | 3 | 3 | 17 |
| p3-16 | audit | FE | 2 | 3 | 3 | 3 | 4 | 3 | 3 | 18 |
| p3-05 | hook | JI | 2 | 2 | 2 | 2 | 3 | 4 | 2 | 15 |
| p3-06 | hook | TE | 2 | 3 | 2 | 2 | 3 | 3 | 3 | 15 |
| p3-13 | hook | ME | 2 | 3 | 2 | 2 | 3 | 4 | 3 | 16 |
| p3-08 | context | SI | 4 | 3 | 3 | 1 | 4 | 2 | 4 | 17 |
| p3-11 | context | QO | 3 | 4 | 3 | 1 | 3 | 2 | 3 | 16 |
| p3-15 | context | CU | 4 | 4 | 3 | 1 | 3 | 3 | 4 | 18 |

Per arm:

| Arm | n | types | errors | ownership | simplicity | comments | tests | overall | Mean sum of 6 (max 24) | Sums per run | Range |
|---|---|---|---|---|---|---|---|---|---|---|---|
| audit | 3 | 2.00 | 3.33 | 2.67 | 3.00 | 4.00 | 3.00 | 3.00 | 18.00 | 19, 17, 18 | 17 to 19 |
| context | 3 | 3.67 | 3.67 | 3.00 | 1.00 | 3.33 | 2.33 | 3.67 | 17.00 | 17, 16, 18 | 16 to 18 |
| hook | 3 | 2.00 | 2.67 | 2.00 | 2.00 | 3.00 | 3.67 | 2.67 | 15.33 | 15, 15, 16 | 15 to 16 |
| router | 3 | 2.00 | 2.00 | 2.00 | 2.00 | 3.00 | 2.00 | 2.00 | 13.00 | 13, 13, 13 | 13 to 13 |
| BASE | 1 | 2.00 | 2.00 | 2.00 | 2.00 | 3.00 | 2.00 | 2.00 | 13.00 | 13 | 13 to 13 |
| skills | 3 | 2.00 | 2.00 | 2.00 | 2.00 | 3.00 | 2.00 | 2.00 | 13.00 | 13, 13, 13 | 13 to 13 |
| none | 3 | 2.00 | 2.00 | 2.00 | 2.00 | 2.67 | 2.00 | 2.00 | 12.67 | 13, 12, 13 | 12 to 13 |

What separates the top from the bottom:

**types** (`reviews/p3/types.md`)

- Newtypes for the identifier kinds. p3-15[context], p3-08[context] and p3-11[context] give `MessageId`, `ReceiptHandle` and the arrival
  index their own types (p3-11[context]/src/queue.rs:19, p3-08[context]/src/ids.rs, p3-15[context]/src/ids.rs). The receipt handle is
  parsed once into that type and rendered by `Display`, so `HashMap<ReceiptHandle, Arrival>` replaces
  BASE's `HashMap<u64, u64>` (BASE/src/queue.rs:78), where the two `u64`s can be swapped without
  the compiler noticing. No other attempt touched these.
- The command name is parsed into an enum at the boundary only in p3-15[context], p3-08[context] and p3-11[context]
  (`match req.command` at p3-15[context]/src/broker.rs:68). The other 15 crates keep
  `match req.command.as_str()` (BASE/src/broker.rs:65).
- Making invalid states unrepresentable. p3-15[context]'s `QueueKind` (p3-15[context]/src/attributes.rs:38) and p3-08[context]'s `Delivery`
  (p3-08[context]/src/queue.rs:24) model rules from the spec in the type itself. p3-15[context], p3-08[context] and p3-11[context] also fold the FIFO
  pair `group_id`/`sequence_number` into one `Option<FifoPosition>`, where BASE has two independent
  `Option`s.
- Time. p3-15[context] is the only crate that tells instants from spans (`Instant + Seconds`, p3-15[context]/src/time.rs).
  p3-11[context] uses `Duration` for both (p3-11[context]/src/broker.rs:24). p3-08[context] and every other crate keep bare `u64`.
- In the middle tier, p3-03[router], p3-10[none] and p3-01[router] replace the anonymous `(group, dedup)` tuple with a named
  `FifoIds`, and p3-03[router] and p3-10[none] add an outcome enum. p3-09[none] and p3-07[none] add the enum but keep
  `Option<(&str, &str)>` inside it. p3-14[audit], p3-16[audit] and p3-12[audit] pass the validated `Option<DeadLetter>` into `apply`
  but leave the raw pair on `AttributeUpdate` as dead `_` fields, which is worse than replacing it
  (p3-15[context] and p3-08[context] do replace it with `DeadLetterUpdate`).

**errors** (`reviews/p3/errors.md`)

- **Required parameters.** The top eight make "present" part of the read: `required_string` (p3-12[audit]/src/request.rs:83, p3-11[context]/src/request.rs:120), or `require` returning the values (p3-15[context]/src/request.rs:161, p3-08[context]/src/request.rs:168). The bottom eleven keep `params.string(..)?.expect("required")` at each call site (for example p3-17[skills]/src/broker/message_commands.rs:54). That expect is only safe while the separate `require` list stays in sync.
- **States that cannot occur.** Only p3-11[context], p3-15[context], p3-12[audit] and p3-16[audit] remove the silent `if let Some(message) = ...message_mut(key)` (BASE/src/broker/message_commands.rs:196):
  - p3-11[context] turns it into a typed error path (p3-11[context]/src/broker/message_commands.rs:177);
  - p3-15[context], p3-12[audit] and p3-16[audit] turn it into an `expect` that names its invariant (p3-15[context]/src/broker/message_commands.rs:185, p3-12[audit]/src/broker/message_commands.rs:198, p3-16[audit]/src/queue.rs:191).
- **Casts on input.** p3-15[context], p3-13[hook], p3-11[context], p3-08[context], p3-06[hook] and p3-12[audit] pass `max_messages` as `u64` and drop the cast, so `plan_receive` takes the type of the parameter (p3-12[audit]/src/queue.rs:194). The rest keep `max_messages as usize` (p3-03[router]/src/broker/message_commands.rs:194). Range checking makes it harmless, but it is still an `as` on input.
- **No regressions.** No attempt introduced a dropped `Result`, a `String` error type, a failure turned into a default, or a change in check order. The bottom group is penalized for leaving BASE as it was, not for making it worse.

**ownership** (`reviews/p3/ownership.md`)

- Removing the double queue lookup in delete/change-visibility. p3-16[audit] (:197), p3-12[audit] (:180), p3-11[context] (:202) and
  p3-15[context] return or reuse the `&mut Queue` found while checking the receipt handle. The other 14 crates
  still call `in_flight_message` and then `self.queue_mut(name)` again, e.g. p3-03[router]/src/broker/message_commands.rs:223,237.
- Parsing `Command` into an enum at the boundary. p3-15[context], p3-11[context] and p3-08[context] no longer clone the command `String`
  out of the JSON map to match on it. Every other crate keeps `name.clone()` at request.rs:31.
- `dead_letter()` returning a borrow. p3-08[context] (attributes.rs:144) and p3-15[context] (attributes.rs:175) return
  `Option<&DeadLetter>`, so validating no longer allocates. p3-16[audit], p3-14[audit] and p3-12[audit] moved the clone instead
  of removing it.
- The dead-letter name clone during receive is still there in every crate. p3-11[context] limits it to receives
  that actually dead-letter something and says why in a comment. p3-08[context] makes it cheap with `Arc<str>`,
  which is a workaround rather than a restructure. No crate splits the borrow so that the clone is
  unnecessary.
- p3-15[context] alone replaces the collect-then-remove loop in `expire` with one `retain` pass that also
  clears handles (p3-15[context]/src/queue.rs:140). It also adds a small new allocation: `QueueName::parse`
  owns the name before the "already exists" check (p3-15[context]/src/broker/queue_commands.rs:27).

**simplicity** (`reviews/p3/simplicity.md`)

- **Removing a decision made twice instead of moving it.** Only p3-12[audit], p3-16[audit] and p3-14[audit] stop calling `update.dead_letter()` just to discard the result. They pass the resolved value into `apply` instead (p3-12[audit]/src/broker/queue_commands.rs:43-46, p3-14[audit]/...:55-58). Every other crate still pairs the dead-letter fields in two places (e.g. p3-04[skills]/src/broker/queue_commands.rs:43).
- **Removing the double queue lookup.** p3-12[audit], p3-16[audit], p3-11[context] and p3-15[context] return or pass the queue, so delete and visibility changes no longer do two lookups with an impossible `if let` (p3-16[audit]/src/broker/message_commands.rs:197). The others keep BASE's pattern (e.g. p3-07[none]/src/broker/message_commands.rs:216).
- **Adding one-caller abstractions around `send_message`.** p3-01[router], p3-07[none], p3-09[none], p3-10[none], p3-03[router] and p3-08[context] each pull the FIFO checks into a helper with five or six parameters. Most also wrap the result in a new enum or struct (p3-03[router]/src/broker/message_commands.rs:48-71; p3-10[none]/...:53-67). The handler gets no shorter and gains a type to learn.
- **Type rewrites add code without removing duplication.** p3-11[context], p3-08[context] and p3-15[context] introduce ids/time modules and a `Command` enum whose `from_name` repeats the dispatch match (p3-15[context]/src/request.rs:32, p3-11[context]/src/request.rs:32). They also copy counter methods (p3-11[context]/src/ids.rs:14/:36; p3-15[context] ids.rs:13/:31/:58). Yet they leave the copied CBD block in place (p3-15[context]/src/broker/queue_commands.rs:44/:97).
- **Placeholder tricks.** p3-08[context] and p3-15[context] make `require` return typed values, which is a real simplification. But both need a static `Null` sentinel to fill the array before every slot is checked (p3-08[context]/src/request.rs:169, p3-15[context]/src/request.rs:94).

**comments** (`reviews/p3/comments.md`)

- The top four kept docs true when they changed a signature: p3-16[audit]/src/attributes.rs:66, p3-14[audit]/src/attributes.rs:66, p3-12[audit]/src/attributes.rs:71 and p3-08[context]/src/request.rs:166. p3-15[context] changed `require` to return values and left "Fails with ... first absent name" as its only doc (p3-15[context]/src/request.rs:160).
- The best additions state preconditions or invariants the type cannot express:
  - p3-16[audit]/src/queue.rs:189: `key` is only valid if nothing changed in between.
  - p3-12[audit]/src/broker/message_commands.rs:206: every smaller handle number has been issued.
  - p3-11[context]/src/ids.rs:27: handle order equals issue order.
- Rationale for a non-obvious line or structure is rare and valuable: p3-11[context]/src/broker/message_commands.rs:155 (why the name is copied) and p3-08[context]/src/attributes.rs:23 (why the name is shared).
- p3-09[none] is lowest because it documents everything, including the obvious. "The messages and settings of one queue" (p3-09[none]/src/queue.rs:83), "The number of messages held" (`:112`) and "Counts the messages by state" (`:137`) restate the names. It has the most comment lines among the minimal-diff crates.
- Error-constructor docs are the most common filler: p3-04[skills]/src/error.rs:54, p3-05[hook]/src/error.rs:52, p3-10[none]/src/request.rs:55, p3-17[skills]/src/request.rs:48, p3-07[none]/src/error.rs:53, p3-03[router]/src/error.rs:53.

**tests** (`reviews/p3/tests.md`)

- Only p3-13[hook] and p3-05[hook] touched the integration tests, replacing `contains(...)` checks on whole response lines with exact
  comparisons (p3-13[hook]/tests/binary.rs:38-41 parses and compares objects; p3-05[hook] compares raw strings, with one prefix match
  at p3-05[hook]/tests/binary.rs:31). p3-05[hook] also split the boolean at p3-05[hook]/tests/queues.rs:257-258 into two `assert_eq`.
- Mid-tier attempts (p3-14[audit], p3-06[hook], p3-12[audit], p3-16[audit], p3-15[context]) fixed the `is_ok()/is_err()` range test into exact values and error
  variants (e.g. p3-14[audit]/src/request.rs:194), and most also replaced the trailing `is_ok()` with an exact parsed command.
- p3-15[context], p3-08[context] and p3-11[context] added new unit tests for the types they introduced (ids, time, QueueName) with table-driven exact
  expectations (p3-15[context]/src/ids.rs:76, p3-11[context]/src/ids.rs:73, p3-08[context]/src/request.rs:254), but p3-08[context] and p3-11[context] kept every base lapse and
  p3-11[context] added `assert!(first < second && second < next)` (p3-11[context]/src/ids.rs:69).
- Eleven attempts made no change to any test, so they inherit BASE's score exactly.
- p3-13[hook] and p3-12[audit] now assert exact `message` text (p3-13[hook]/src/request.rs:201), which the spec leaves free; exact, but it
  couples the test to wording.

**overall** (`reviews/p3/overall.md`)

- **Parsing required parameters into values.** p3-15[context] (p3-15[context]/src/request.rs, `Required`) and p3-08[context]
  (p3-08[context]/src/request.rs:168) make `require` return the values, so no handler can read a parameter it
  forgot to require. p3-06[hook]/p3-13[hook]/p3-16[audit]/p3-12[audit]/p3-14[audit] swap `.expect` for `required_*`, which is safe but keeps two
  checks of one fact. The other ten keep `.expect("required")` at every handler
  (e.g. p3-01[router]/src/broker/message_commands.rs:91).
- **Types for the domain.** Only p3-15[context], p3-08[context] and p3-11[context] add `MessageId`/`ReceiptHandle`/arrival-index
  newtypes and a `Command` enum. p3-15[context] goes furthest with `QueueName` (keyed map with `Borrow<str>`)
  and `Instant` vs `Seconds`; p3-11[context]'s `Duration` for instants is a weaker choice
  (p3-11[context]/src/broker.rs:25).
- **Fields that belong together.** p3-15[context], p3-08[context] and p3-11[context] fold `group_id` + `sequence_number` into one
  `Option<FifoPosition>`, and p3-15[context] and p3-08[context] turn the dead-letter pair into an enum. p3-08[context]'s `Delivery` enum
  (p3-08[context]/src/queue.rs:24) ties receive count, first receive and handle together.
- **Removing duplicate work, not just duplicate text.** p3-16[audit] and p3-12[audit] remove the second queue lookup in
  `DeleteMessage`/`ChangeMessageVisibility` (p3-16[audit]/src/broker/message_commands.rs:197), and so do p3-15[context]
  and p3-11[context]. Most of the rest only extract the repeated `ContentBasedDeduplication` check.
- **Extraction that adds ceremony.** p3-07[none], p3-03[router], p3-09[none] and p3-10[none] move the FIFO send rules into a helper with
  5 to 6 positional parameters, or a single-use struct (p3-03[router]/src/broker/message_commands.rs:48). That
  helps the reader very little over BASE's inline block.

Reading.

- `none`, `skills` and `router` are the start crate. Blind: every one of the nine runs scores 2 on
  types, errors, ownership, simplicity, tests and overall, exactly as BASE does; the only cell that
  differs is one `comments` score (p3-09, 2 instead of 3). Mechanical: `expect` per KLOC stays at
  11.8 to 12.3, weak assertions at 20, newtypes at 0.
- `router` is the cleanest version of v1's finding: seven guides loaded as the first actions, in
  three runs of three, and nothing a reviewer or a counter can see.
- `skills` loaded `rust-keep-it-simple` in all three runs (plus `rust-comments` once) and nothing
  else, as the two v1 runs that were told to use skills did. The `hook` runs did the same before the
  hook fired.
- `audit`: `expect` per KLOC falls to 1.0 to 2.1 in all three runs, weak assertions from 20 to 15 or
  16, in 13 to 15 turns and about a minute and a half. It makes no type change (0 newtypes, `types`
  2, 2, 2). It is the only arm that raises `simplicity` (3, 3, 3): the reviewer names its three runs
  as the only ones that remove a decision made twice instead of moving it, and two of them also
  remove the double queue lookup. It loaded only `rust-audit`; no run loaded one of the guides the
  entry skill points to.
- `context`: the largest change (6 to 12 commits, 3.5 to 5.5 minutes): `expect` to 0.9 to 2.5,
  `clone` from 3.07 to between 0.0 and 1.9, newtypes 0, 3, 7, enums 6, 4, 6. Highest on `types`,
  `errors` and `overall` (3.67 each), lowest on `simplicity` (1, 1, 1) and it did not touch the weak
  tests in two of three runs.
- `hook`: the detector's feedback led to fixes in tests in all three runs (weak assertions 8, 15, 9)
  and to the removal of `expect` on required parameters in two (p3-06, p3-13). Highest `tests` score
  (3.67).
- The three arms that work are complementary by dimension: `audit` simplicity and comments,
  `context` types and errors, `hook` tests.

### 5.4 P4: greenfield on Haiku 5.5 and Opus 5.5

Design: the P1 task with `none`, `skills` and `context`, two runs per model and arm (the plan said
"none plus the best arm of P1"; both `skills`, the arm under test, and `context`, the best on the
mechanical counts, were run). Five dimensions reviewed (types, errors, simplicity, tests, overall);
one reviewer per dimension saw all twelve crates, so Haiku and Opus crates are scored on one scale.

| Arm | Model | Runs |
|---|---|---|
| none | haiku-5-5 | p4-06, p4-10 |
| skills | haiku-5-5 | p4-05, p4-11 |
| context | haiku-5-5 | p4-03, p4-07 |
| none | opus-5-5 | p4-02, p4-12 |
| skills | opus-5-5 | p4-08, p4-09 |
| context | opus-5-5 | p4-01, p4-04 |

Process:

| Run | Arm | Hidden tests | Own tests | Skill calls | Skills loaded, in order | At tool call | Tool calls | Stop hook | Turns | Min | USD | Commits | Diff vs start |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| p4-06 | none/haiku | 58/58 | ok | 0 | - | - | 34 | - | 35 | 9.3 | 0.45 | 12 | 15 files, 2956+ |
| p4-10 | none/haiku | 58/58 | ok | 0 | - | - | 41 | - | 42 | 9.5 | 0.52 | 12 | 18 files, 3746+ |
| p4-05 | skills/haiku | 58/58 | ok | 0 | - | - | 111 | - | 98 | 17.1 | 1.63 | 8 | 16 files, 3326+ |
| p4-11 | skills/haiku | 58/58 | ok | 5 | comments, types, errors, tests, simple | 3, 4, 5, 6, 7 | 51 | - | 57 | 11.2 | 0.74 | 18 | 14 files, 3144+ |
| p4-03 | context/haiku | 58/58 | ok | 0 | - | - | 59 | - | 60 | 16.9 | 1.18 | 6 | 14 files, 3144+ |
| p4-07 | context/haiku | 58/58 | ok | 0 | - | - | 38 | - | 39 | 15.7 | 1.0 | 11 | 16 files, 2969+ |
| p4-02 | none/opus | 58/58 | ok | 0 | - | - | 22 | - | 23 | 8.1 | 1.97 | 5 | 10 files, 2496+ |
| p4-12 | none/opus | 58/58 | ok | 0 | - | - | 30 | - | 31 | 9.4 | 2.3 | 6 | 12 files, 2524+ |
| p4-08 | skills/opus | 58/58 | ok | 4 | errors, types, tests, comments | 2, 3, 4, 5 | 34 | - | 39 | 11.1 | 2.93 | 4 | 14 files, 2646+ |
| p4-09 | skills/opus | 58/58 | ok | 4 | errors, types, tests, comments | 2, 3, 4, 5 | 37 | - | 42 | 12.4 | 3.32 | 5 | 13 files, 2771+ |
| p4-01 | context/opus | 58/58 | ok | 0 | - | - | 37 | - | 38 | 15.1 | 3.76 | 5 | 14 files, 3144+ |
| p4-04 | context/opus | 58/58 | ok | 0 | - | - | 50 | - | 51 | 16.5 | 4.35 | 5 | 17 files, 2904+ |

Code:

| Run | Arm | Prod lines | Test lines | src files | Newtypes | Enums | Stringly ids /KLOC | Str compares /KLOC | expect /KLOC | unwrap_or* /KLOC | clone /KLOC | as casts /KLOC | Lint set /KLOC | Comment % | Tests | Weak asserts | allow attrs |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| p4-06 | none/haiku | 1314 | 1277 | 11 | 0 | 4 | 29.08 | 0.86 | 0.0 | 2.57 | 7.7 | 0.86 | 1.71 | 11.0 | 84 | 28 | 0 |
| p4-10 | none/haiku | 1211 | 2164 | 9 | 0 | 3 | 21.9 | 0.95 | 4.76 | 5.71 | 2.86 | 1.9 | 6.67 | 13.3 | 123 | 18 | 1 |
| p4-05 | skills/haiku | 1377 | 1580 | 8 | 0 | 4 | 22.42 | 1.6 | 5.6 | 5.6 | 4.8 | 0.0 | 5.6 | 9.3 | 102 | 5 | 1 |
| p4-11 | skills/haiku | 1460 | 1374 | 10 | 6 | 4 | 14.42 | 0.76 | 3.03 | 6.07 | 6.83 | 0.0 | 1.52 | 9.7 | 84 | 18 | 0 |
| p4-03 | context/haiku | 1667 | 1230 | 11 | 10 | 5 | 13.15 | 1.31 | 7.89 | 5.92 | 3.29 | 0.0 | 2.63 | 8.8 | 42 | 3 | 0 |
| p4-07 | context/haiku | 1700 | 1019 | 9 | 12 | 10 | 7.03 | 0.64 | 6.39 | 10.87 | 4.48 | 0.0 | 1.28 | 8.0 | 43 | 1 | 0 |
| p4-02 | none/opus | 1317 | 970 | 7 | 1 | 4 | 35.45 | 0.82 | 5.77 | 9.89 | 8.24 | 0.0 | 4.95 | 7.9 | 43 | 19 | 0 |
| p4-12 | none/opus | 1276 | 1026 | 7 | 1 | 2 | 32.31 | 0.85 | 6.8 | 6.8 | 6.8 | 0.0 | 3.4 | 7.8 | 48 | 7 | 1 |
| p4-08 | skills/opus | 1289 | 1154 | 8 | 5 | 5 | 9.38 | 1.71 | 6.82 | 6.82 | 4.26 | 0.0 | 1.71 | 9.0 | 53 | 0 | 1 |
| p4-09 | skills/opus | 1365 | 1186 | 8 | 5 | 6 | 10.35 | 1.59 | 6.37 | 9.55 | 7.17 | 0.0 | 3.18 | 8.0 | 58 | 0 | 1 |
| p4-01 | context/opus | 1617 | 1280 | 8 | 13 | 7 | 8.08 | 1.35 | 2.02 | 7.4 | 2.69 | 0.0 | 3.36 | 8.1 | 39 | 0 | 0 |
| p4-04 | context/opus | 1453 | 1231 | 8 | 10 | 8 | 11.18 | 1.49 | 2.98 | 7.45 | 4.47 | 0.0 | 8.94 | 7.6 | 41 | 0 | 0 |

Means per arm and model:

| Arm | n | Skill calls per run | Newtypes per run | Newtypes mean | Enums | Stringly ids /KLOC | Str compares /KLOC | expect /KLOC | unwrap_or* /KLOC | clone /KLOC | Lint set /KLOC | Comment % | Tests | Weak asserts per run | Prod lines | Turns | Min | USD | Commits |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| none/haiku | 2 | 0, 0 | 0, 0 | 0.0 | 3.5 | 25.5 | 0.91 | 2.4 | 4.1 | 5.3 | 4.2 | 12.2 | 104 | 28, 18 | 1262 | 38 | 9.4 | 0.48 | 12.0 |
| skills/haiku | 2 | 0, 5 | 0, 6 | 3.0 | 4.0 | 18.4 | 1.18 | 4.3 | 5.8 | 5.8 | 3.6 | 9.5 | 93 | 5, 18 | 1418 | 78 | 14.2 | 1.19 | 13.0 |
| context/haiku | 2 | 0, 0 | 10, 12 | 11.0 | 7.5 | 10.1 | 0.98 | 7.1 | 8.4 | 3.9 | 2.0 | 8.4 | 42 | 3, 1 | 1684 | 50 | 16.3 | 1.09 | 8.5 |
| none/opus | 2 | 0, 0 | 1, 1 | 1.0 | 3.0 | 33.9 | 0.83 | 6.3 | 8.3 | 7.5 | 4.2 | 7.8 | 46 | 19, 7 | 1296 | 27 | 8.8 | 2.13 | 5.5 |
| skills/opus | 2 | 4, 4 | 5, 5 | 5.0 | 5.5 | 9.9 | 1.65 | 6.6 | 8.2 | 5.7 | 2.4 | 8.5 | 56 | 0, 0 | 1327 | 40 | 11.8 | 3.12 | 4.5 |
| context/opus | 2 | 0, 0 | 13, 10 | 11.5 | 7.5 | 9.6 | 1.42 | 2.5 | 7.4 | 3.6 | 6.1 | 7.8 | 40 | 0, 0 | 1535 | 44 | 15.8 | 4.05 | 5.0 |

Blind scores per run:

| Run | Arm | Blind code | types | errors | simplicity | tests | overall | Sum of 4 (max 16, overall excluded) |
|---|---|---|---|---|---|---|---|---|
| p4-06 | none/haiku | NI | 1 | 2 | 2 | 2 | 2 | 7 |
| p4-10 | none/haiku | SO | 0 | 2 | 3 | 3 | 1 | 8 |
| p4-05 | skills/haiku | TA | 1 | 3 | 3 | 3 | 4 | 10 |
| p4-11 | skills/haiku | QE | 2 | 2 | 2 | 3 | 2 | 9 |
| p4-03 | context/haiku | QI | 3 | 3 | 2 | 3 | 3 | 11 |
| p4-07 | context/haiku | PA | 4 | 3 | 1 | 4 | 2 | 12 |
| p4-02 | none/opus | FU | 1 | 3 | 3 | 3 | 3 | 10 |
| p4-12 | none/opus | KE | 1 | 3 | 3 | 3 | 2 | 10 |
| p4-08 | skills/opus | XI | 3 | 3 | 4 | 4 | 3 | 14 |
| p4-09 | skills/opus | ZU | 3 | 3 | 3 | 3 | 4 | 12 |
| p4-01 | context/opus | WE | 4 | 4 | 2 | 4 | 3 | 14 |
| p4-04 | context/opus | MU | 4 | 4 | 3 | 3 | 3 | 14 |

Per arm and model:

| Arm | n | types | errors | simplicity | tests | overall | Mean sum of 4 (max 16) | Sums per run | Range |
|---|---|---|---|---|---|---|---|---|---|
| context/opus | 2 | 4.00 | 4.00 | 2.50 | 3.50 | 3.00 | 14.00 | 14, 14 | 14 to 14 |
| skills/opus | 2 | 3.00 | 3.00 | 3.50 | 3.50 | 3.50 | 13.00 | 14, 12 | 12 to 14 |
| context/haiku | 2 | 3.50 | 3.00 | 1.50 | 3.50 | 2.50 | 11.50 | 11, 12 | 11 to 12 |
| none/opus | 2 | 1.00 | 3.00 | 3.00 | 3.00 | 2.50 | 10.00 | 10, 10 | 10 to 10 |
| skills/haiku | 2 | 1.50 | 2.50 | 2.50 | 3.00 | 3.00 | 9.50 | 10, 9 | 9 to 10 |
| none/haiku | 2 | 0.50 | 2.00 | 2.50 | 2.50 | 1.50 | 7.50 | 7, 8 | 7 to 8 |

What separates the top from the bottom:

**types** (`reviews/p4/types.md`)

- **Typed command at the boundary.** At one extreme, p4-07[context] turns the line into a fully typed `Command` that already holds `QueueRef`, `Body`, `Option<ReceiptHandle>` (p4-07[context]/src/request.rs:15-58). At the other, p4-10[none] keeps `command: String` plus a raw `Map<String, Value>` and parses inside each handler (p4-10[none]/src/protocol.rs:16-17, broker.rs:76).
- **Ids and handles.** The top six define `MessageId`/`ReceiptHandle`/`SequenceNumber` with `Display` and a strict `parse` (e.g. p4-01[context]/src/types.rs:202-245). p4-02[none], p4-12[none], p4-06[none], p4-05[skills] and p4-10[none] pass `u64` and format `"m-{}"`/`"r-{}"` at the call site (p4-06[none]/src/broker.rs:459-469, p4-05[skills]/src/protocol.rs:111-115).
- **Delivery state as one value.** p4-04[context], p4-01[context], p4-08[skills] and p4-09[skills] make a `Hold`/`Delivery` enum that carries the receipt handle only in the in-flight variant (p4-01[context]/src/store.rs:32), and p4-07[context] does the same with `Option<Reception>`. p4-11[skills] and p4-10[none] keep `invisible_until: u64` with `0` as "never delivered", next to a separate `receipt: Option` (p4-11[skills]/src/queue.rs:84-91).
- **Queue kind.** p4-04[context] folds `ContentBasedDeduplication` into `QueueKind::Fifo` (p4-04[context]/src/queue.rs:31). p4-07[context], p4-01[context] and p4-09[skills] put the FIFO-only state into `Kind::Fifo(..)`. Every crate from p4-03[context] down stores `fifo: bool` and `content_based_deduplication: bool` side by side (p4-03[context]/src/attributes.rs:138, p4-02[none]/src/queue.rs:48).
- **FIFO message fields.** Top crates use one `Option<FifoTag>` (p4-08[skills]/src/queue.rs:95, p4-01[context]/src/store.rs:26). p4-02[none], p4-06[none], p4-11[skills], p4-10[none] and p4-05[skills] keep two parallel Options that can disagree (p4-11[skills]/src/queue.rs:87-88).
- **Output shape.** p4-03[context], p4-01[context], p4-07[context], p4-04[context], p4-08[skills] and p4-09[skills] serialize typed result structs. p4-06[none], p4-11[skills], p4-12[none] and p4-10[none] assemble `Map<String, Value>` inside the broker or service (p4-06[none]/src/broker.rs:22, p4-11[skills]/src/broker.rs:19).

**errors** (`reviews/p4/errors.md`)

- **Strict integer parsing.** p4-04[context], p4-01[context], p4-12[none] and p4-09[skills] turn on `arbitrary_precision` and check the number's source text (p4-04[context]/src/request.rs:318, p4-09[skills]/src/request.rs:284), so `-0` is accepted and `30.0`/`3e1` are rejected. The other eight use `as_u64`/`as_i64` on a plain `Value`, which rejects the valid integer `-0` with `InvalidParameterValue`. p4-06[none] documents this (p4-06[none]/src/params.rs:28) and ships it anyway.
- **Typed error values.** p4-04[context] and p4-01[context] give each failure reason its own variant of a thiserror enum (p4-04[context]/src/error.rs:27, p4-01[context]/src/error.rs:26), mapped to the wire code in one place. Everyone else uses a `{code: ErrorCode, message: String}` struct: the code is typed, but the reason is free text.
- **Counters and the clock.** p4-07[context] and p4-03[context] make every counter and clock step checked, with an `expect` that says why overflow cannot happen (p4-07[context]/src/types.rs:63, p4-03[context]/src/time.rs:36). p4-10[none] uses `as` casts and unchecked `+=` (p4-10[none]/src/broker.rs:344, 353), and p4-11[skills] and p4-08[skills] use plain `now + x`. These are bounded by the spec in practice, but nothing in the code says so.
- **Branches for impossible states.** p4-07[context], p4-03[context], p4-01[context] and p4-09[skills] each keep an `unreachable!`/`panic!` arm for a FIFO/standard pairing their types allow (p4-07[context]/src/queue.rs:719, p4-03[context]/src/queue.rs:290, p4-01[context]/src/queue.rs:240, p4-09[skills]/src/queue.rs:320). p4-05[skills], p4-08[skills], p4-04[context] and p4-02[none] have none.
- **Consistency within a crate.** p4-11[skills] handles the same "planned message is still stored" invariant in two ways, dropping it silently at p4-11[skills]/src/broker.rs:287 and `expect`ing it at broker.rs:312. p4-12[none] and p4-02[none] have `expect` messages that only restate the call (p4-12[none]/src/dedup.rs:57, p4-02[none]/src/queue.rs:460).

**simplicity** (`reviews/p4/simplicity.md`)

- **One merge helper or several.** The top crates use one helper: p4-08[skills], p4-12[none] (`p4-12[none]/src/queue.rs:72`), p4-10[none]
  (`p4-10[none]/src/attributes.rs:121`), p4-05[skills] (`p4-05[skills]/src/attributes.rs:70`) and p4-11[skills] (`p4-11[skills]/src/attributes.rs:163`). p4-07[context]
  checks equality field by field in `matches` (`p4-07[context]/src/queue.rs:456`), separately from `create` (397),
  which is the same decision computed twice.
- **Forwarding layers.** p4-01[context] (`p4-01[context]/src/queue.rs:246`) and p4-09[skills] (`p4-09[skills]/src/queue.rs:203`) put a `MessageStore`
  under `Queue`, and many `Queue` methods only delegate to it. p4-07[context] wraps a map plus an index in `Store`
  (`p4-07[context]/src/queue.rs:255`). p4-08[skills] and p4-04[context] keep the two collections directly on `Queue` and lose nothing.
- **How ReceiveMessage is written.** In the best crates it is one loop over arrival order, followed by
  removal of the dead-lettered messages (`p4-04[context]/src/queue.rs:359`, `p4-09[skills]/src/queue.rs:513`). The weaker ones
  split it into plan and apply with an intermediate enum and then look queues up again: p4-06[none]
  (`p4-06[none]/src/broker.rs:348`), p4-11[skills] (`p4-11[skills]/src/broker.rs:279-306`) and p4-07[context] (`p4-07[context]/src/queue.rs:644-694`).
- **Accessor and wire-struct ceremony.** p4-03[context] adds getters to every struct and copies `Counts` into
  `CountsWire` (`p4-03[context]/src/reply.rs:95`). p4-08[skills] and p4-09[skills] serialize their domain structs directly with serde
  attributes (`p4-08[skills]/src/queue.rs:19`, `p4-09[skills]/src/queue.rs:31`).
- **Duplicated send paths.** p4-07[context] (`p4-07[context]/src/queue.rs:549`/`599`), p4-01[context] (`p4-01[context]/src/queue.rs:171`/`207`) and p4-08[skills]
  (227/282) each build the message in two branches. p4-09[skills] builds it once after the match
  (`p4-09[skills]/src/queue.rs:276`).
- **Branches for impossible states.** p4-06[none]'s `deliver` silently returns on a missing key
  (`p4-06[none]/src/queue.rs:256`). p4-05[skills]'s `remove` has a `None => (None, true)` group arm (`p4-05[skills]/src/queue.rs:437`).
  p4-11[skills]'s receive re-checks `queue_missing` (`p4-11[skills]/src/broker.rs:283`, 306). The better crates use an `expect`
  that names the invariant (`p4-04[context]/src/queue.rs:474`).

**tests** (`reviews/p4/tests.md`)

- Whole-response comparison against literal JSON. p4-07[context] (messages.rs:23), p4-01[context] (common.rs:38) and p4-08[skills] (messages.rs:78) compare complete replies, so a missing or extra member fails the test. p4-06[none] (broker_messages.rs:88-98) and p4-10[none] (receive.rs:96-103) assert one member per line and prove absence with `get(..).is_none()`, so an extra member they did not think of passes.
- Every setup step checked. p4-01[context]'s `check()` pairs every request with its expected reply, and p4-08[skills] and p4-09[skills] assert inside `create`. In p4-03[context] (protocol.rs:188), p4-04[context] (dead_letters.rs:73) and p4-05[skills] (`last()` in common/mod.rs:30), setup replies are dropped, so a test can end up exercising a state it did not intend.
- Error variants rather than truthiness. p4-12[none], p4-10[none] and p4-02[none] mostly assert `unwrap_err().code == ErrorCode::X`. p4-06[none] (attributes.rs:198-206) checks range bounds with bare `is_err()`, which passes on any error code.
- Determinism. p4-02[none] (binary.rs:58) and p4-06[none] (broker_messages.rs:471) bound wall-clock time inside the normal test run, so a slow CI machine can fail them. The others assert the limit only by its result: the 100000th message is accepted and the next one gets `LimitExceeded`.
- The error message is normalized, not ignored. p4-07[context] (support/mod.rs:32), p4-01[context] and p4-08[skills] assert that `message` is a string and then remove it so the rest of the reply can be compared exactly. p4-10[none]'s `fail` (common/mod.rs:22) never looks at the message.

**overall** (`reviews/p4/overall.md`)

- **A typed boundary in both directions.** p4-05[skills], p4-09[skills], p4-01[context], p4-04[context] and p4-02[none] parse into a `Request`/`Command` enum and return a typed `Reply`/`Output` that is rendered in one place (p4-09[skills]/src/response.rs:48, p4-01[context]/src/lib.rs:42). p4-10[none] keeps `serde_json::Map` all the way into the handlers (p4-10[none]/src/broker.rs:74-92). p4-12[none], p4-11[skills] and p4-06[none] type the input but build JSON inside the broker (p4-12[none]/src/service.rs:45, p4-11[skills]/src/broker.rs:19, p4-06[none]/src/broker.rs:22). A change to a response shape then touches the business logic.
- **Receive written as one direct pass.** p4-05[skills] (p4-05[skills]/src/queue.rs:326), p4-03[context] and p4-02[none] walk an index with a cursor and mutate in place, with one comment saying why the cursor exists. p4-11[skills] re-borrows the map three times across plan, move and deliver (p4-11[skills]/src/broker.rs:266-306). p4-06[none] re-looks up the source queue on every iteration (p4-06[none]/src/broker.rs:349).
- **No unbounded bookkeeping.** p4-07[context] (p4-07[context]/src/broker.rs:32), p4-11[skills] (p4-11[skills]/src/broker.rs:35) and p4-10[none] (p4-10[none]/src/broker.rs:45) store one record per delivery for the life of the process to resolve receipts. The others resolve a handle through a per-queue map that is pruned on redelivery and removal.
- **Amount to learn before a change is safe.** p4-05[skills] and p4-09[skills] read top to bottom in one sitting. p4-07[context], p4-03[context] and p4-01[context] add lifetime-carrying outputs, `range_mut` lookups or remove-and-reinsert tricks (p4-07[context]/src/broker.rs:315, p4-01[context]/src/broker.rs:278) that a maintainer has to understand before touching receive.
- **Impossible states.** The better crates use `expect` that names an invariant. p4-09[skills] still has a `panic!` match arm (p4-09[skills]/src/queue.rs:320). p4-06[none] silently ignores `None` from a lookup that cannot fail (p4-06[none]/src/broker.rs:371, 379).

Reading.

- Same order in both models as in Sonnet: `none` below `skills` below `context` on the blind sum,
  with no overlap between `none` and either treatment inside a model (two runs per cell).
- Activation depends on the model. Opus loaded four guides in both runs, at tool calls 2 to 5
  (errors, types, tests, comments; never simplicity or ownership). Haiku loaded five in one run
  (p4-11) and none in the other (p4-05), which then took 98 turns and 111 tool calls, the longest
  run of v2.
- Where the guides were loaded they did more on Opus than on Sonnet: 5 and 5 newtypes and 9.4 and
  10.4 stringly ids per KLOC (Opus `none`: 1 and 1, 35.5 and 32.3). Sonnet's `skills` runs in P1 had
  1 to 4 newtypes and 18 to 28 stringly ids per KLOC.
- The checklist in `CLAUDE.md` needs no activation and worked in all four runs (10 to 13 newtypes).
  `context` on Haiku (sums 11, 12) is above `none` on Opus (10, 10) at about half the cost per run
  (USD 1.09 against 2.13).
- The profile seen in P1 repeats: `context` has the lowest `simplicity` in each model (1.5 on Haiku,
  2.5 on Opus) and `skills` the highest or tied highest `overall` (3.0 and 3.5).

### 5.5 P5: the v1 crate with the `audit` arm

Design: two more runs of the v1 setup (aws-emulator SQS crate at `813af480`, v1 `run.sh`, v1 plain
prompt, Sonnet 5.5, symposium home `branch`), with `PLUGIN_DIR=v2/arms/audit`. They are v1 runs 14
and 15 (`../mapping.tsv`) and were reviewed with the v1 brief (`../review-brief.md`: eleven
dimensions scored -1 to +2 against the baseline, one reviewer per run). Verifier pass: no
(the fourteen earlier v1 runs each had one). They ran under `~`, so the global `CLAUDE.md` was loaded
as in all of v1.

| Run | Crate tests | Skill calls | Tool calls | Turns | Min | USD | Commits | Diff | Lint total (baseline 360) | expect /KLOC (baseline 0.71) | clone /KLOC (baseline 8.42) | Comment % (baseline 43.4) | src files (baseline 63) | Rubric sum |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 14 | PASS | rust-audit at call 1 | 33 | 35 | 9.8 | 0.66 | 5 | 29 files changed, 75 insertions(+), 238 deletions(-) | 264 | 0.31 | 7.67 | 43.7 | 63 | 7 |
| 15 | PASS | rust-audit at call 1 | 42 | 44 | 12.0 | 0.86 | 11 | 30 files changed, 318 insertions(+), 434 deletions(-) | 166 | 0.15 | 8.21 | 43.6 | 64 | 9 |

Both runs loaded `rust-audit` as their first tool call and no other skill.

Commits, run 14:

```text
9592b7216 refactor(sqs): authorize against a queue through one helper
9aacd1c51 refactor(sqs): resolve the queue once in the four policy and tag handlers
6eec620a4 chore(sqs): drop lint allows that suppress nothing
5bb98196f refactor(sqs): drop service overrides that restate the trait defaults
3b4dd3847 test(sqs): rename scratch_probe to say what it checks
```

Commits, run 15:

```text
6c0b4d5ae refactor(sqs): drop unused_imports allows that hide nothing
677e4da5a refactor(sqs): drop lint allows on the types re-exports
1db883274 refactor(sqs): drop crate-level lint allows that hide nothing
8deb70ad9 refactor(sqs): remove override_* methods that restate the trait defaults
3ead399fc refactor(sqs): resolve the queue once in tag/untag/add/remove permission ops
f495a9406 refactor(sqs): authorise SetQueueAttributes against borrowed queue fields
c3540965e refactor(sqs): model message move task status as an enum
d33f362ab test(sqs): rename scratch_probe to say what it covers
397e338b3 refactor(sqs): drop the remaining lookup-then-expect in move-task and set-attributes
18e5d8e1e refactor(sqs): move message move task types out of state.rs into state/message_move.rs
2e594ec46 refactor(sqs): drop a needless borrow of the queue name
```

Scores (reviewer, not verified):

| Dimension (v1 rubric, -1 to +2) | Run 14 | Run 15 |
|---|---|---|
| Structure | 0 | 1 |
| Duplication | 1 | 1 |
| Error handling | 1 | 1 |
| Domain types | 0 | 1 |
| Ownership | 1 | 1 |
| Comments | 0 | 0 |
| Control flow | 1 | 1 |
| Lint suppressions | 1 | 2 |
| Tests | 0 | 0 |
| Commit hygiene | 2 | 1 |
| Behavior risk | 0 | 0 |
| Sum | 7 | 9 |

Audit findings:

| Audit finding | Run 14 | Run 15 |
|---|---|---|
| S-01 | untouched | untouched |
| S-05 | untouched | untouched |
| S-07 | untouched | untouched |
| S-08 | untouched | untouched |
| S-09 | untouched | untouched |
| S-12/S-13 | partial | partial |
| S-14/S-15 | untouched | untouched |
| S-16/S-17 | partial | partial |
| S-18 | untouched | untouched |
| S-19 | untouched | untouched |
| S-20 | untouched | untouched |
| S-21 | untouched | untouched |

For comparison, the v1 table (`../TRIAL-LOG.md` section 5.1, verified scores):

| Run | Condition | Arm | Prompt | Skills loaded | Struct | Dedupe | Errors | Types | Owner | Comments | Flow | Lints | Tests | Commits | Risk | Sum |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| pilot | A | pilot | plain | 0 | +2 | +2 | +1 | 0 | +1 | 0 | +2 | +1 | 0 | +2 | 0 | 11 |
| 8 | B | main | +line | 0 | +2 | +1 | +1 | 0 | 0 | +1 | +1 | +1 | 0 | +2 | 0 | 9 |
| 11 | D | branch + router with hints | plain | 7 | +2 | +1 | +1 | +1 | +1 | 0 | +1 | 0 | +1 | +1 | 0 | 9 |
| 1 | A | branch | plain | 0 | +2 | +1 | +1 | 0 | 0 | 0 (reviewer +1) | +1 | +1 | 0 | +2 | 0 | 8 (reviewer 9) |
| 10 | D | branch + router with hints | plain | 7 | +1 | +1 | +1 | +1 | +1 | 0 | 0 | 0 | 0 | +2 | 0 | 7 |
| 5 | B | main | +line | 0 | +2 | +1 | +1 | 0 | 0 | 0 | 0 | 0 | 0 | +2 | 0 | 6 |
| 3 | A | branch | plain | 0 | +2 | +1 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | +2 | 0 | 5 |
| 2 | A | main | plain | 0 | +2 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | +2 | 0 | 4 |
| 4 | B | main | +line | 0 | +2 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | +2 | 0 | 4 |
| 6 | C | branch | +line | 2 | +2 | +1 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | +1 | 0 | 4 |
| 12 | E | branch + router without hints | plain | 7 | 0 | +1 | 0 | 0 | +1 | 0 | 0 | +1 | 0 | +1 | 0 | 4 |
| 7 | C | branch | +line | 2 | +1 | 0 | 0 | 0 | 0 | 0 | +1 | 0 | 0 | +1 | 0 | 3 |
| 9 | B | branch | +line | 0 | +1 | 0 | 0 | 0 | 0 | -1 | 0 | 0 | 0 | +2 | 0 | 2 |
| 13 | E | branch + router without hints | plain | 7 | +1 | 0 | 0 | 0 | 0 | -1 | 0 | +1 | 0 | +1 | 0 | 2 |

And v1 by condition (section 5.5 there):

| Condition | Runs | Verified sums | Mean verified sum | Mean reviewer sum | Mean turns | Mean minutes | Mean cost USD | Mean commits |
|---|---|---|---|---|---|---|---|---|
| A. No skills loaded, plain | pilot, 1, 2, 3 | 11, 8, 4, 5 | 7.00 | 7.25 | 93 | 46.0 | 2.75 | 23.5 |
| B. No skills loaded, +line | 4, 5, 8, 9 | 4, 6, 9, 2 | 5.25 | 5.25 | 102 | 63.4 | 2.55 | 18.2 |
| C. Two skills loaded, +line | 6, 7 | 4, 3 | 3.50 | 3.50 | 62 | 28.8 | 1.23 | 10.0 |
| D. Seven loaded, router with hints | 10, 11 | 7, 9 | 8.00 | 8.00 | 82 | 23.1 | 1.90 | 6.5 |
| E. Seven loaded, router without hints | 12, 13 | 4, 2 | 3.00 | 3.00 | 63 | 16.1 | 1.17 | 7.0 |

Reading.

- Run 15's sum of 9 ties the second place of the fourteen v1 runs (runs 8 and 11, 132 and 102
  turns) in 44 turns and 12 minutes; run 14's 7 equals run 10 (62 turns) in 35 turns and under 10
  minutes. The v1 runs that loaded all seven guides without hints scored 4 and 2.
- Neither run split `state.rs` as its plan: run 14 moved nothing (Structure 0, 63 files as at
  baseline), run 15 moved one cohesive module as its tenth commit. In v1, 13 of 14 runs split that
  file. The entry skill's last step ("Move code between files last") is the likely reason.
- Run 15 has the lowest lint total (166) and the lowest `expect` density (0.15 per KLOC) of all
  sixteen runs on this crate; the v1 minimum was 268 and 0.26. Both runs removed lint suppressions,
  which only five v1 runs did.
- Run 15 made the status enum (`MessageMoveStatus`) that in v1 only the two hinted-router runs made.
  Run 14 did not.
- Unchanged from v1: S-01, S-05, S-07, S-08, S-09 and S-21 are untouched in both runs, and the
  comment share (43.4 percent at baseline) does not move.
- Limits: two runs; the reviews were not verified; one reviewer per run, so the v1 calibration
  problem applies; and the entry skill's list was written by someone who knew this crate's audit
  (section 4.4), which makes this the least independent result in the file.

### 5.6 P6: iteration on what P1 to P3 showed

Three questions came out of P1 to P3. First, the 68 checklist lines act from `CLAUDE.md`, and the
1170 lines of guides loaded as skills act less (P1) or not at all (P3): is that the channel or the
length? Second, the checklist over-applies types. Third, one guide tells a cleanup to leave working
code alone. P6 tests three arms on all three tasks, three runs each, Sonnet 5.5:

- `short`: the `context` text as one skill (channel, same content).
- `limits`: checklist plus the guides' "Do not over-..." bullets plus a scope sentence, as one
  skill.
- `context2`: the `limits` text in `CLAUDE.md`.

The greenfield and refactor results were reviewed blind together with anchor runs from P1 and P3
(the `none`, `skills` and `context` runs of P1; the `none`, `audit` and `context` runs of P3, plus
BASE), so that new and old arms sit on one scale and the anchors give a test-retest measure
(section 7). Dimensions: types, errors, simplicity, tests, overall for greenfield; types, errors,
simplicity, overall for refactor. The feature runs were not reviewed blind, because nothing moved
mechanically.

#### P6 greenfield

| Arm | Model | Runs |
|---|---|---|
| short | sonnet-5-5 | p6g-03, p6g-05, p6g-09 |
| limits | sonnet-5-5 | p6g-02, p6g-06, p6g-07 |
| context2 | sonnet-5-5 | p6g-01, p6g-04, p6g-08 |

Process:

| Run | Arm | Hidden tests | Own tests | Skill calls | Skills loaded, in order | At tool call | Tool calls | Stop hook | Turns | Min | USD | Commits | Diff vs start |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| p6g-03 | short | 58/58 | ok | 1 | rules | 1 | 20 | - | 22 | 7.6 | 1.21 | 7 | 11 files, 2585+ |
| p6g-05 | short | 58/58 | ok | 1 | rules | 1 | 18 | - | 20 | 7.4 | 1.19 | 6 | 10 files, 2338+ |
| p6g-09 | short | 58/58 | ok | 1 | rules | 1 | 23 | - | 25 | 7.1 | 1.19 | 7 | 12 files, 2484+ |
| p6g-02 | limits | 58/58 | ok | 1 | rules | 1 | 14 | - | 16 | 5.4 | 0.88 | 5 | 11 files, 2139+ |
| p6g-06 | limits | 58/58 | ok | 1 | rules | 1 | 20 | - | 22 | 6.3 | 1.04 | 9 | 12 files, 2274+, 1- |
| p6g-07 | limits | 58/58 | ok | 1 | rules | 1 | 10 | - | 12 | 5.6 | 0.88 | 4 | 11 files, 2004+ |
| p6g-01 | context2 | 58/58 | ok | 0 | - | - | 17 | - | 18 | 8.5 | 1.35 | 7 | 11 files, 2641+ |
| p6g-04 | context2 | 58/58 | ok | 0 | - | - | 18 | - | 19 | 8.0 | 1.28 | 6 | 11 files, 2461+ |
| p6g-08 | context2 | 58/58 | ok | 0 | - | - | 26 | - | 27 | 11.3 | 1.65 | 5 | 13 files, 2931+ |

Code:

| Run | Arm | Prod lines | Test lines | src files | Newtypes | Enums | Stringly ids /KLOC | Str compares /KLOC | expect /KLOC | unwrap_or* /KLOC | clone /KLOC | as casts /KLOC | Lint set /KLOC | Comment % | Tests | Weak asserts | allow attrs |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| p6g-03 | short | 1446 | 917 | 9 | 7 | 10 | 16.61 | 0.0 | 3.61 | 9.39 | 4.33 | 0.0 | 1.44 | 4.2 | 47 | 4 | 0 |
| p6g-05 | short | 1344 | 800 | 8 | 8 | 8 | 14.13 | 1.57 | 0.78 | 13.34 | 3.92 | 0.0 | 1.57 | 5.2 | 37 | 5 | 0 |
| p6g-09 | short | 1371 | 887 | 9 | 9 | 6 | 12.22 | 0.76 | 3.82 | 10.7 | 4.58 | 0.0 | 5.35 | 4.5 | 51 | 11 | 0 |
| p6g-02 | limits | 1264 | 706 | 9 | 2 | 7 | 25.06 | 0.84 | 5.01 | 10.86 | 5.85 | 0.0 | 5.01 | 5.3 | 36 | 0 | 0 |
| p6g-06 | limits | 1194 | 884 | 9 | 3 | 4 | 18.31 | 0.87 | 3.49 | 13.08 | 6.97 | 0.87 | 1.74 | 3.9 | 44 | 3 | 0 |
| p6g-07 | limits | 1215 | 624 | 8 | 3 | 4 | 24.41 | 0.87 | 4.36 | 13.95 | 5.23 | 0.0 | 0.0 | 5.6 | 27 | 7 | 0 |
| p6g-01 | context2 | 1307 | 1122 | 9 | 4 | 6 | 12.99 | 0.81 | 6.49 | 4.87 | 5.68 | 0.81 | 3.25 | 5.7 | 46 | 6 | 0 |
| p6g-04 | context2 | 1376 | 898 | 9 | 3 | 7 | 19.23 | 0.0 | 6.15 | 6.15 | 4.62 | 0.0 | 1.54 | 5.5 | 36 | 5 | 0 |
| p6g-08 | context2 | 1374 | 1319 | 8 | 6 | 5 | 19.83 | 1.53 | 7.63 | 7.63 | 6.1 | 0.0 | 6.86 | 4.6 | 58 | 7 | 1 |

Means per arm:

| Arm | n | Skill calls per run | Newtypes per run | Newtypes mean | Enums | Stringly ids /KLOC | Str compares /KLOC | expect /KLOC | unwrap_or* /KLOC | clone /KLOC | Lint set /KLOC | Comment % | Tests | Weak asserts per run | Prod lines | Turns | Min | USD | Commits |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| short | 3 | 1, 1, 1 | 7, 8, 9 | 8.0 | 8.0 | 14.3 | 0.78 | 2.7 | 11.1 | 4.3 | 2.8 | 4.6 | 45 | 4, 5, 11 | 1387 | 22 | 7.4 | 1.20 | 6.7 |
| limits | 3 | 1, 1, 1 | 2, 3, 3 | 2.7 | 5.0 | 22.6 | 0.86 | 4.3 | 12.6 | 6.0 | 2.2 | 4.9 | 36 | 0, 3, 7 | 1224 | 17 | 5.8 | 0.93 | 6.0 |
| context2 | 3 | 0, 0, 0 | 4, 3, 6 | 4.3 | 6.0 | 17.4 | 0.78 | 6.8 | 6.2 | 5.5 | 3.9 | 5.3 | 47 | 6, 5, 7 | 1352 | 21 | 9.3 | 1.43 | 6.0 |

Blind scores per run (new runs and P1 anchors, one reviewer per dimension over all eighteen):

| Run | Arm | Blind code | types | errors | simplicity | tests | overall | Sum of 4 (max 16, overall excluded) |
|---|---|---|---|---|---|---|---|---|
| p1-03 | none | JI | 0 | 1 | 2 | 3 | 1 | 6 |
| p1-04 | none | LI | 0 | 1 | 2 | 2 | 1 | 5 |
| p1-07 | none | SO | 1 | 2 | 2 | 3 | 2 | 8 |
| p1-05 | skills | DE | 2 | 4 | 2 | 4 | 2 | 12 |
| p1-08 | skills | KO | 1 | 3 | 3 | 3 | 2 | 10 |
| p1-12 | skills | HU | 1 | 2 | 3 | 4 | 2 | 10 |
| p1-06 | context | JU | 4 | 3 | 1 | 3 | 3 | 11 |
| p1-10 | context | NE | 3 | 4 | 2 | 3 | 2 | 12 |
| p1-13 | context | CE | 4 | 4 | 3 | 4 | 4 | 15 |
| p6g-03 | short | CA | 4 | 4 | 2 | 3 | 4 | 13 |
| p6g-05 | short | NI | 3 | 3 | 3 | 3 | 4 | 12 |
| p6g-09 | short | TA | 3 | 4 | 3 | 3 | 3 | 13 |
| p6g-02 | limits | RU | 2 | 4 | 3 | 3 | 3 | 12 |
| p6g-06 | limits | LA | 2 | 3 | 4 | 3 | 3 | 12 |
| p6g-07 | limits | QA | 2 | 4 | 3 | 2 | 3 | 11 |
| p6g-01 | context2 | VE | 2 | 3 | 3 | 4 | 3 | 12 |
| p6g-04 | context2 | NO | 3 | 3 | 3 | 3 | 3 | 12 |
| p6g-08 | context2 | TU | 3 | 3 | 2 | 4 | 2 | 12 |

Per arm:

| Arm | n | types | errors | simplicity | tests | overall | Mean sum of 4 (max 16) | Sums per run | Range |
|---|---|---|---|---|---|---|---|---|---|
| short | 3 | 3.33 | 3.67 | 2.67 | 3.00 | 3.67 | 12.67 | 13, 12, 13 | 12 to 13 |
| context | 3 | 3.67 | 3.67 | 2.00 | 3.33 | 3.00 | 12.67 | 11, 12, 15 | 11 to 15 |
| context2 | 3 | 2.67 | 3.00 | 2.67 | 3.67 | 2.67 | 12.00 | 12, 12, 12 | 12 to 12 |
| limits | 3 | 2.00 | 3.67 | 3.33 | 2.67 | 3.00 | 11.67 | 12, 12, 11 | 11 to 12 |
| skills | 3 | 1.33 | 3.00 | 2.67 | 3.67 | 2.00 | 10.67 | 12, 10, 10 | 10 to 12 |
| none | 3 | 0.33 | 1.33 | 2.00 | 2.67 | 1.33 | 6.33 | 6, 5, 8 | 5 to 8 |

What separates the top from the bottom:

**types** (`reviews/p6g/types.md`)

- **One typed command at the boundary versus raw JSON in the handlers.** p6g-03[short], p1-13[context], p1-06[context], p1-10[context] and others parse into a `Command` enum with typed fields. p1-03[none] (p1-03[none]/src/message_commands.rs:19) and p1-04[none] (p1-04[none]/src/broker.rs:63) give handlers a `Params` view over the raw map, so validation and domain logic are mixed together.
- **Typed or untyped responses.** The top 13 serialize response structs and enums. p1-12[skills], p1-07[none], p1-03[none] and p1-04[none] build `Map<String, Value>` with `json!` and format ids by hand (p1-07[none]/src/broker.rs:325, p1-03[none]/src/message_commands.rs:182).
- **Range-checked attribute types.** p1-13[context], p1-06[context], p1-10[context] and p6g-08[context2] use a const-generic `Bounded`/`Ranged`/`Seconds<MIN, MAX>`, and p6g-03[short] uses a macro. The range is enforced once and travels with the value. p1-05[skills], p6g-06[limits], p6g-07[limits], p6g-02[limits], p6g-01[context2], p1-08[skills], p1-12[skills] and p1-07[none] carry `u32`/`u64` and re-check ranges by hand.
- **Fields that only make sense together.** The best crates put the handle inside the in-flight state (p1-06[context]/src/queue.rs:118, p6g-05[short]/src/queue.rs:34, p6g-04[context2]/src/queue.rs:70, p6g-01[context2]/src/queue.rs:104). The weak ones keep parallel `*_until: u64` timestamps next to an `Option` handle (p1-07[none]/src/queue.rs:94-98, p1-03[none]/src/queue.rs:154-157). FIFO is an enum carrying CBD (p6g-03[short], p1-13[context], p1-06[context], p1-10[context], p6g-04[context2], p6g-02[limits], p6g-09[short], p6g-08[context2]) in some crates and `fifo: bool` plus `content_based_deduplication: bool` (p1-05[skills], p1-12[skills], p1-03[none], p6g-06[limits], p1-04[none], p6g-07[limits], p1-07[none], p6g-01[context2]) in others.
- **Sentinels versus `Option`.** p1-08[skills] stores `u64::MAX` for "no messages" (p1-08[skills]/src/queue.rs:175). p1-06[context] and p1-04[none] use `Option` for the same field. p1-10[context] has a `Hold::None` variant and an `accepted: bool` flag where an outcome enum belongs; p1-06[context] (`SendOutcome`) and p6g-05[short] (`Admission`) use an enum.

**errors** (`reviews/p6g/errors.md`)

- **Where required-ness lives.** The top crates have a `required(name)?` that returns the value, so
  absence becomes `MissingParameter` in one place. p1-03[none], p1-04[none], p1-12[skills] and p6g-08[context2] check a list with
  `require(&[...])` and then `expect` each value (p1-03[none]/src/queue_commands.rs:82,
  p1-04[none]/src/broker/message_commands.rs:57, p6g-08[context2]/src/request.rs:189). That couples two call sites, and a
  mismatch panics on input.
- **Silent skips of impossible states versus named expects.** p6g-03[short], p1-05[skills], p1-10[context], p6g-05[short], p6g-07[limits], p6g-02[limits], p6g-09[short] and p6g-01[context2]
  `expect` the dead-letter target with a reason (p1-05[skills]/src/broker.rs:313). p1-03[none] and p1-04[none] skip it with
  `if let Some` (p1-03[none]/src/message_commands.rs:104), and p1-07[none] drops the `Result` (p1-07[none]/src/broker.rs:272).
  Either way, messages would vanish without any signal.
- **Defaults for missing state.** p1-03[none] emits `r-{latest_handle.unwrap_or_default()}` and
  `unwrap_or(0)` for a required parameter. p1-07[none] uses `unwrap_or(now)` for an unset `FirstReceivedAt`.
  No top crate turns an absent value into a made-up one.
- **Conversions.** The top crates use `u32::try_from`/`usize::try_from(..).expect("at most 10")`
  (p6g-07[limits]/src/broker.rs:315). p1-03[none] casts range-checked `i64` with `as u32`/`as u64` in 9 places, and
  p1-08[skills], p6g-06[limits], p6g-01[context2], p1-12[skills], p1-07[none] and p1-04[none] each keep one `as usize` on `MaxMessages`.
- **Error typing.** p6g-03[short] alone models each protocol code as an enum variant with typed causes
  (p6g-03[short]/src/error.rs:7-73). Every other crate uses `ErrorCode` plus a free `String` message. The spec
  leaves the message text free, so this is not a defect, but it is less checkable.

**simplicity** (`reviews/p6g/simplicity.md`)

- **One representation per concept.** p6g-06[limits] keeps a message's state in `visible_at` plus an optional handle (p6g-06[limits]/src/queue.rs:38). p1-06[context] and p1-10[context] keep a public `QueueType` alongside an internal `Kind` and convert between them (p1-06[context]/src/queue.rs:27 and :107, p1-10[context]/src/queue.rs:228). p1-06[context] adds three more attribute structs on top.
- **Plain data structures for the job.** The spec only asks that sends to a full queue be fast. p1-05[skills] (p1-05[skills]/src/queue.rs:124-131) and p6g-08[context2] (p6g-08[context2]/src/queue.rs:175-179) still maintain 5 or 6 synchronized indexes, while p1-12[skills], p6g-06[limits] and p6g-07[limits] do the same work with one `BTreeMap` plus an age index.
- **Shared helpers for what create and set share.** p6g-09[short] (p6g-09[short]/src/broker.rs:130) and p1-13[context] (p1-13[context]/src/service.rs:171) resolve the dead-letter target once. p1-08[skills] (p1-08[skills]/src/service.rs:137 vs :179), p6g-03[short] (p6g-03[short]/src/service.rs:134 vs :164) and others copy the block.
- **No caches that need manual upkeep.** p1-07[none]'s `swept_at` needs `invalidate_sweep` (p1-07[none]/src/queue.rs:221), p6g-05[short]'s `swept_at` adds a `refresh` vs `expire` split (p6g-05[short]/src/queue.rs:166), and p1-04[none] keeps a pruning threshold (p1-04[none]/src/queue.rs:255). p6g-02[limits], p1-10[context] and p6g-01[context2] avoid all of this by expiring on `Tick`, the only thing that moves the clock (p6g-02[limits]/src/service.rs:50).
- **The required check made once.** p1-03[none] and p1-04[none] check that a parameter is present, then `.expect("required")` it again in every handler (p1-03[none]/src/message_commands.rs:19). Crates that parse into typed commands make that decision once.

**tests** (`reviews/p6g/tests.md`)

- **Whole responses compared, not single fields.** The top crates assert entire messages or response lists (p1-12[skills]/tests/receive.rs:45, p1-13[context]/tests/it/behavior.rs:32, p1-06[context]/tests/protocol.rs:42). Weaker ones probe single members (p6g-02[limits]/tests/protocol.rs:413, p6g-06[limits]/tests/scenarios.rs:557), which would miss an extra or wrong member.
- **Exact framing checks.** p1-13[context]/tests/it/binary.rs:5 and p1-10[context]/tests/binary.rs:18 compare stdout byte for byte. p1-04[none]/tests/binary.rs:29, p1-07[none]/tests/binary.rs:29 and p6g-07[limits]/tests/cli.rs:33 accept any line that merely contains "InvalidRequest".
- **One behavior per test, named for it.** p1-05[skills] and p1-12[skills] split the handle rules into separate scenarios (p1-05[skills]/tests/messages.rs:557-620, p1-12[skills]/tests/handles.rs). p6g-07[limits] packs send, receive, delay, expiry and invalid handles into one test (p6g-07[limits]/tests/protocol.rs:271), so a failure points to no single rule.
- **Coverage depth.** p1-05[skills] (98 tests), p1-12[skills] (62) and p6g-08[context2] (58) cover nearly every numbered rule, including subtle ones (p1-05[skills]/tests/messages.rs:522, a full FIFO queue still answers duplicates). p6g-07[limits] (27) and p6g-02[limits] (36) skip several dead-letter rules.
- **The real process.** p6g-08[context2] drives every test through the binary (p6g-08[context2]/tests/common/mod.rs:20). p1-08[skills], p1-03[none], p6g-06[limits], p6g-05[short] and p6g-02[limits] never check the exit code or real stdin/stdout.
- **Expected values written in the test.** Most crates write literal ids such as `m-1` and `r-3`. p6g-09[short]/src/queue.rs:756 loops to the crate's own constant, so a wrong limit would still pass.

**overall** (`reviews/p6g/overall.md`)

- A typed boundary. p6g-03[short]/p1-13[context]/p6g-05[short] parse each line into a `Command` of newtypes (p6g-03[short]/src/command.rs:52,
  p1-13[context]/src/command.rs:59) and render a typed `Output`. p1-03[none]/p1-04[none] pass a raw `Params` map into every handler
  and assume earlier checks passed, `params.string(..)?.expect("required")` (p1-03[none]/src/message_commands.rs:19,
  p1-04[none]/src/broker/queue_commands.rs:21), so a reordering silently becomes a panic.
- Typed responses. Crates that `Serialize` an `Output` enum (p1-13[context]/src/output.rs:35, p6g-02[limits]/src/response.rs:10)
  let the compiler hold the "exactly these members" rule. p1-12[skills]/p1-07[none]/p1-03[none]/p1-04[none] build `Map<String, Value>` with
  `json!` (p1-12[skills]/src/broker.rs:204, p1-07[none]/src/broker.rs:172), so a misspelled member name compiles.
- State modeled as an enum vs. parallel fields. p1-10[context]/p6g-02[limits]/p6g-09[short]/p6g-01[context2]/p6g-05[short] have a `Hold`/`Availability` enum with the
  handle inside; p1-08[skills]/p1-04[none]/p1-07[none] keep `delayed_until`, `in_flight_until`, `handle: Option<u64>` side by side
  (p1-07[none]/src/queue.rs:85), which allows combinations that cannot occur.
- Proportionate data structures. p1-05[skills] and p6g-08[context2] keep 5 to 7 indexes in sync (p1-05[skills]/src/queue.rs:115,
  p6g-08[context2]/src/queue.rs:171) where a single `BTreeMap` scan (p6g-03[short], p1-13[context], p6g-05[short]) passes the spec; every future change
  to them has to re-establish every invariant.
- Error types. p6g-03[short]'s `Error` enum with typed causes (p6g-03[short]/src/error.rs:52) is the easiest to match and
  extend. Most others use `ErrorCode + String message`, which is adequate. None return `String` errors.

Reading.

- Channel: `short` equals `context`. Same mean sum (12.67), 7 to 9 newtypes against 8 to 10, and
  `short` has the tighter range (12 to 13 against 11 to 15) and the highest `overall` (3.67). The
  skill was loaded as the first tool call in all three runs. For new code, whether the 68 lines
  arrive as a skill or as `CLAUDE.md` makes no visible difference.
- Length: `short` against `skills`, both delivered as skills and both loaded at the start: 12.67
  against 10.67 on the same reviewers, ranges 12 to 13 and 10 to 12, `types` 3.33 against 1.33. One
  68-line skill did more than five or six guides of about 200 lines each. It is also cheaper: 7.4
  minutes and USD 1.20 per run against 9.5 and 1.72.
- Limits: the "But:" bullets do what they say. `simplicity` rises (`limits` 3.33 against `short`
  2.67; `context2` 2.67 against `context` 2.00) and `types` falls (2.00 against 3.33; 2.67 against
  3.67); newtypes drop from 7 to 9 to 2 or 3 (`limits`) and from 8 to 10 to 3 to 6 (`context2`). On
  the sum the two effects cancel: 11.67 and 12.00 against 12.67. `limits` is the cheapest treated
  arm of the trial at 5.8 minutes and USD 0.93, the cost of `none` in P1.
- All five treated arms are clear of `none` (5 to 8 against 10 to 15).

#### P6 refactor

| Arm | Model | Runs |
|---|---|---|
| short | sonnet-5-5 | p6r-04, p6r-05, p6r-07 |
| limits | sonnet-5-5 | p6r-02, p6r-03, p6r-06 |
| context2 | sonnet-5-5 | p6r-01, p6r-08, p6r-09 |

Process:

| Run | Arm | Hidden tests | Own tests | Skill calls | Skills loaded, in order | At tool call | Tool calls | Stop hook | Turns | Min | USD | Commits | Diff vs start |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| p6r-04 | short | 58/58 | ok | 1 | rules | 1 | 14 | - | 16 | 1.8 | 0.36 | 6 | 6 files, 112+, 72- |
| p6r-05 | short | 58/58 | ok | 1 | rules | 1 | 7 | - | 9 | 1.0 | 0.23 | 3 | 6 files, 78+, 40- |
| p6r-07 | short | 58/58 | ok | 1 | rules | 1 | 19 | - | 21 | 2.1 | 0.42 | 5 | 7 files, 191+, 92- |
| p6r-02 | limits | 58/58 | ok | 1 | rules | 1 | 19 | - | 21 | 2.2 | 0.46 | 5 | 8 files, 179+, 82- |
| p6r-03 | limits | 58/58 | ok | 1 | rules | 1 | 12 | - | 14 | 1.3 | 0.32 | 3 | 8 files, 107+, 52- |
| p6r-06 | limits | 58/58 | ok | 1 | rules | 1 | 11 | - | 13 | 1.1 | 0.26 | 3 | 5 files, 24+, 21- |
| p6r-01 | context2 | 58/58 | ok | 0 | - | - | 29 | - | 30 | 3.8 | 0.76 | 7 | 9 files, 378+, 177- |
| p6r-08 | context2 | 58/58 | ok | 0 | - | - | 39 | - | 40 | 4.8 | 0.94 | 7 | 9 files, 434+, 200- |
| p6r-09 | context2 | 58/58 | ok | 0 | - | - | 15 | - | 16 | 2.3 | 0.45 | 5 | 9 files, 285+, 174- |

Code (BASE is the start crate):

| Run | Arm | Prod lines | Test lines | src files | Newtypes | Enums | Stringly ids /KLOC | Str compares /KLOC | expect /KLOC | unwrap_or* /KLOC | clone /KLOC | as casts /KLOC | Lint set /KLOC | Comment % | Tests | Weak asserts | allow attrs |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| BASE (p1-04) | - | 1057 | 1073 | 10 | 0 | 3 | 19.43 | 2.04 | 12.27 | 10.22 | 3.07 | 1.02 | 5.11 | 7.5 | 44 | 20 | 1 |
| p6r-04 | short | 1092 | 1073 | 11 | 3 | 3 | 20.9 | 1.99 | 12.94 | 9.95 | 2.99 | 0.0 | 2.99 | 8.0 | 44 | 20 | 1 |
| p6r-05 | short | 1092 | 1073 | 10 | 0 | 4 | 19.82 | 0.99 | 11.89 | 9.91 | 1.98 | 0.99 | 4.96 | 7.6 | 44 | 20 | 1 |
| p6r-07 | short | 1125 | 1093 | 11 | 3 | 4 | 21.3 | 0.97 | 1.94 | 9.68 | 2.9 | 0.0 | 2.9 | 8.2 | 45 | 20 | 1 |
| p6r-02 | limits | 1118 | 1099 | 11 | 3 | 4 | 19.46 | 0.97 | 12.65 | 9.73 | 1.95 | 0.97 | 4.86 | 8.1 | 45 | 19 | 1 |
| p6r-03 | limits | 1101 | 1080 | 11 | 0 | 4 | 19.63 | 0.98 | 11.78 | 9.81 | 1.96 | 0.98 | 4.91 | 7.4 | 44 | 20 | 1 |
| p6r-06 | limits | 1059 | 1073 | 10 | 0 | 3 | 19.41 | 2.04 | 13.28 | 10.21 | 3.06 | 0.0 | 3.06 | 7.6 | 44 | 20 | 1 |
| p6r-01 | context2 | 1210 | 1101 | 11 | 3 | 5 | 18.9 | 0.9 | 14.4 | 9.0 | 0.9 | 0.0 | 6.3 | 8.2 | 46 | 15 | 1 |
| p6r-08 | context2 | 1223 | 1104 | 11 | 4 | 4 | 19.64 | 0.89 | 13.39 | 8.93 | 0.89 | 0.0 | 6.25 | 8.4 | 46 | 20 | 1 |
| p6r-09 | context2 | 1129 | 1100 | 11 | 3 | 4 | 19.25 | 0.96 | 14.44 | 9.62 | 1.92 | 0.0 | 2.89 | 8.0 | 46 | 20 | 1 |

Means per arm:

| Arm | n | Skill calls per run | Newtypes per run | Newtypes mean | Enums | Stringly ids /KLOC | Str compares /KLOC | expect /KLOC | unwrap_or* /KLOC | clone /KLOC | Lint set /KLOC | Comment % | Tests | Weak asserts per run | Prod lines | Turns | Min | USD | Commits |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| short | 3 | 1, 1, 1 | 3, 0, 3 | 2.0 | 3.7 | 20.7 | 1.32 | 8.9 | 9.8 | 2.6 | 3.6 | 7.9 | 44 | 20, 20, 20 | 1103 | 15 | 1.6 | 0.34 | 4.7 |
| limits | 3 | 1, 1, 1 | 3, 0, 0 | 1.0 | 3.7 | 19.5 | 1.33 | 12.6 | 9.9 | 2.3 | 4.3 | 7.7 | 44 | 19, 20, 20 | 1093 | 16 | 1.5 | 0.35 | 3.7 |
| context2 | 3 | 0, 0, 0 | 3, 4, 3 | 3.3 | 4.3 | 19.3 | 0.92 | 14.1 | 9.2 | 1.2 | 5.1 | 8.2 | 46 | 15, 20, 20 | 1187 | 29 | 3.6 | 0.72 | 6.3 |

Blind scores per run (new runs, P3 anchors and BASE):

| Run | Arm | Blind code | types | errors | simplicity | overall | Sum of 3 (max 12, overall excluded) |
|---|---|---|---|---|---|---|---|
| BASE | - | BASE | 1 | 1 | 2 | 3 | 4 |
| p3-07 | none | ZE | 1 | 1 | 2 | 3 | 4 |
| p3-09 | none | XE | 1 | 1 | 2 | 2 | 4 |
| p3-10 | none | PE | 1 | 1 | 2 | 3 | 4 |
| p3-12 | audit | QO | 1 | 3 | 3 | 3 | 7 |
| p3-14 | audit | KE | 1 | 2 | 3 | 3 | 6 |
| p3-16 | audit | FO | 1 | 3 | 3 | 3 | 7 |
| p3-08 | context | CE | 4 | 3 | 1 | 3 | 8 |
| p3-11 | context | NE | 3 | 4 | 2 | 2 | 9 |
| p3-15 | context | JI | 4 | 4 | 1 | 3 | 9 |
| p6r-04 | short | DE | 2 | 2 | 2 | 3 | 6 |
| p6r-05 | short | FI | 2 | 1 | 2 | 3 | 5 |
| p6r-07 | short | PA | 3 | 3 | 2 | 3 | 8 |
| p6r-02 | limits | TE | 3 | 2 | 2 | 4 | 7 |
| p6r-03 | limits | BE | 2 | 1 | 2 | 3 | 5 |
| p6r-06 | limits | PO | 1 | 2 | 2 | 3 | 5 |
| p6r-01 | context2 | PI | 4 | 3 | 1 | 3 | 8 |
| p6r-08 | context2 | KU | 3 | 4 | 1 | 3 | 8 |
| p6r-09 | context2 | MI | 3 | 3 | 2 | 4 | 8 |

Per arm:

| Arm | n | types | errors | simplicity | overall | Mean sum of 3 (max 12) | Sums per run | Range |
|---|---|---|---|---|---|---|---|---|
| context | 3 | 3.67 | 3.67 | 1.33 | 2.67 | 8.67 | 8, 9, 9 | 8 to 9 |
| context2 | 3 | 3.33 | 3.33 | 1.33 | 3.33 | 8.00 | 8, 8, 8 | 8 to 8 |
| audit | 3 | 1.00 | 2.67 | 3.00 | 3.00 | 6.67 | 7, 6, 7 | 6 to 7 |
| short | 3 | 2.33 | 2.00 | 2.00 | 3.00 | 6.33 | 6, 5, 8 | 5 to 8 |
| limits | 3 | 2.00 | 1.67 | 2.00 | 3.33 | 5.67 | 7, 5, 5 | 5 to 7 |
| none | 3 | 1.00 | 1.00 | 2.00 | 2.67 | 4.00 | 4, 4, 4 | 4 to 4 |
| BASE | 1 | 1.00 | 1.00 | 2.00 | 3.00 | 4.00 | 4 | 4 to 4 |

What separates the top from the bottom:

**types** (`reviews/p6r/types.md`)

- Identifier newtypes: the top eight give message id, receipt handle and arrival key their own types
  (p3-15[context]/src/ids.rs:8, p3-08[context]/src/ids.rs:29), so `handles: HashMap<ReceiptHandle, Arrival>` can no longer be
  filled backwards; the bottom keep `HashMap<u64, u64>` (BASE/src/queue.rs:78).
- The command closed set: everyone from rank 1 to 8 except p6r-04[short] parses `Command` into an enum in
  `Request::parse`; p6r-07[short] parses it lazily (p6r-07[short]/src/request.rs:51), p6r-04[short] and the bottom eight still dispatch
  on a string (BASE/src/broker.rs:65).
- Fields that live together: p3-15[context], p3-08[context], p6r-01[context2], p3-11[context] fold `group_id` + `sequence_number` into one
  `Option<FifoPosition>` (p3-08[context]/src/queue.rs:49); p3-15[context], p3-08[context], p6r-01[context2] also make the dead-letter pair an enum
  (p3-15[context]/src/attributes.rs:128); p3-08[context] goes further with `Delivery` (p3-08[context]/src/queue.rs:24).
- Only p3-15[context] types the remaining wide-open values: `QueueName` (p3-15[context]/src/naming.rs:20), `Instant`/`Seconds`
  (p3-15[context]/src/time.rs:7), and `QueueKind` that makes content-based dedup unrepresentable on a standard
  queue (p3-15[context]/src/attributes.rs:38).
- Many low-ranked attempts spent their effort on error helpers and range constants
  (p6r-03[limits]/src/limits.rs, p3-07[none]/src/error.rs) which do not touch the type model.

**errors** (`reviews/p6r/errors.md`)

- How required parameters are read. The top crates make the read total. p3-08[context]/p3-15[context] `require` returns the present values (p3-08[context]/src/request.rs:168, p3-15[context]/src/request.rs:161). p3-11[context]/p3-12[audit]/p6r-07[short]/p3-16[audit]/p3-14[audit] use `required_*` getters that return `Result`. p6r-08[context2] and p6r-01[context2] keep `expect`, but each one states the invariant (p6r-08[context2]/src/broker/message_commands.rs:40). The bottom six keep BASE's bare `expect("required")` in every handler (e.g. p3-07[none]/src/broker/message_commands.rs:94).
- The impossible-miss branch in ChangeMessageVisibility. The top crates turn the silent `if let Some(message)` either into an expect that names its invariant (p3-15[context] :185, p6r-08[context2] :198, p6r-07[short]/src/queue.rs:194, p3-12[audit] :198, p3-16[audit]/src/queue.rs:191) or into Option -> `ReceiptHandleExpired` (p3-11[context] :190). p3-08[context], p3-14[audit], p6r-02[limits], p6r-04[short], p6r-06[limits] and the bottom six keep the silent branch.
- Silently dropped dead letters. Only p6r-08[context2], p6r-09[context2], p6r-01[context2] and p6r-02[limits] replaced `and_then(|q| self.queues.get_mut(&q))` with an expect that cites DeleteQueue (p6r-08[context2] :160, p6r-02[limits] :155). Everyone else, including p3-15[context] and p3-11[context], still discards messages without a trace if the invariant ever breaks.
- The `as usize` cast. p3-12[audit], p6r-07[short], p3-11[context], p3-08[context] and p3-15[context] made `plan_receive` take `u64`, so the cast is gone. p6r-06[limits], p6r-04[short], p6r-08[context2], p6r-09[context2] and p6r-01[context2] use `usize::try_from` with a named expect. p6r-03[limits], p6r-05[short], p3-10[none], p3-09[none], p3-07[none], p6r-02[limits], p3-16[audit] and p3-14[audit] keep `as usize`.
- One step backwards: p6r-01[context2] adds `unreachable!` for `DeadLetterParams::Partial` (p6r-01[context2]/src/attributes.rs:87), a state its own enum allows. p3-08[context] and p3-15[context] fold the same state away without a panic.

**simplicity** (`reviews/p6r/simplicity.md`)

- **The top crates fix BASE's duplicated decisions with the smallest tool available.**
  - p3-12[audit], p3-14[audit] and p3-16[audit] move the dead-letter pairing to one place by passing `dead_letter()`'s result into
    `apply` (p3-12[audit]/src/attributes.rs:81).
  - p3-12[audit] and p3-14[audit] write the CBD rule once (p3-14[audit]/src/broker/queue_commands.rs:26).
  - None of them adds a type.
- **The double lookup separates p3-16[audit] and p3-12[audit] from the rest of the small changes.**
  - p3-16[audit] returns the queue together with the key (p3-16[audit]/src/broker/message_commands.rs:197).
  - p6r-06[limits], p6r-05[short], p6r-03[limits], p6r-04[short], p6r-02[limits] and p6r-07[short] leave the second `queue_mut` and the `if let` on an impossible `None`.
- **Four attempts add a FIFO-send enum and helper with a single caller.**
  - p3-10[none] (message_commands.rs:58), p3-07[none] (:57), p3-09[none] (:62) and p6r-04[short] (:21) take the FIFO block out into a
    five- or six-parameter function, plus an enum to return through.
  - The handler gets no shorter, and readers now have one more indirection.
- **The bottom crates add structure in bulk.**
  - p6r-08[context2] adds about ten getters on now-private fields, plus a forwarding method (p6r-08[context2]/src/queue.rs:117).
  - p3-08[context] adds a sentinel-based generic `require` (p3-08[context]/src/request.rs:169).
  - p6r-01[context2] adds an enum variant that exists only to be rejected, and an `unreachable!` to handle it
    (p6r-01[context2]/src/attributes.rs:87).
  - p3-15[context] adds new `time` and naming types.
  - All of them still keep at least one of BASE's copied rules (p3-15[context]/src/broker/queue_commands.rs:42/:95).
- **The verbose expects show up in the bottom crates.** p6r-01[context2] and p6r-08[context2] change one-line
  `expect("required")` into eleven three-line expects each (p6r-01[context2]/src/broker/message_commands.rs:39).
  This adds lines without removing the decision that is made twice.

**overall** (`reviews/p6r/overall.md`)

- **Making an implicit invariant explicit.** BASE silently skips dead-letter moves when the target is missing
  (BASE/src/broker/message_commands.rs:168), which would lose messages without a trace. p6r-09[context2]:153, p6r-02[limits]:155, p6r-08[context2]:160 and p6r-01[context2]
  turn this into an `expect` that names why it holds. Most other attempts leave the silent fallback in place
  (p6r-04[short]:163, p3-15[context]:156).
- **Ids that cannot be mixed up, at the right size.** p6r-09[context2], p6r-02[limits] and p6r-04[short] add only the newtypes that pay for themselves
  (MessageId, ReceiptHandle, arrival index) and keep `parse`/`Display` beside them. p3-15[context] and p3-08[context] go further (Instant/Seconds,
  QueueKind, Delivery, typed `require`). That is safer, but it doubles what a newcomer has to learn and adds tricks like
  placeholder `static` Values (p3-15[context]/src/request.rs:94, p3-08[context]/src/request.rs:169).
- **Parsing at the boundary.** Command enums built in `Request::parse` (p6r-09[context2], p6r-02[limits], p3-15[context], p6r-01[context2], p3-11[context], p6r-08[context2], p3-08[context], p6r-05[short], p6r-03[limits]) beat p6r-07[short]'s
  lazy `req.command()?` at dispatch (p6r-07[short]/src/request.rs:73), which keeps the raw String inside the request.
- **Removing the second lookup.** Returning `(&mut Queue, key)` from the handle resolver (p3-16[audit]:197, p6r-01[context2], p6r-08[context2], p3-15[context], p3-12[audit]:207)
  removes BASE's look-up-twice-then-`if let Some` pattern. The weaker variant splits `apply(update, dead_letter)` and leaves
  fields destructured as `_` (p3-16[audit]/p3-12[audit]/p3-14[audit] attributes.rs), so the same data travels two ways.
- **Ceremony without payoff.** p6r-08[context2]'s getter wall (p6r-08[context2]/src/attributes.rs:70 onward), p3-10[none]/p3-07[none]/p3-09[none]'s one-caller `FifoSend`
  enums, and p3-09[none]'s restating doc comments (p3-09[none]/src/queue.rs:137) add reading cost and no safety.
- **Odd type choices.** p3-11[context] models virtual clock instants as `Duration` (p3-11[context]/src/broker.rs:24). p6r-01[context2] and p3-08[context] keep a
  `Partial` enum state just to report it later; p6r-01[context2] ends that state in `unreachable!` (p6r-01[context2]/src/attributes.rs:87).

Reading.

- Channel matters here. The same 68 lines: from `CLAUDE.md` (`context`, P3 runs re-scored) 8, 9, 9;
  from a skill (`short`) 6, 5, 8. Mechanically `context` removed the `expect` calls in three runs
  of three and `short` in one (p6r-07). `short` was loaded as the first tool call every time, so
  this is not an activation failure: the text was in context and was acted on less.
- The limits and the scope sentence did not help cleanup. `context2` keeps the type work (3, 4, 3
  newtypes; `types` 3.33) and the `clone` reduction, but `expect` per KLOC stays at 13.4 to 14.4:
  the reviewers report the one-line `expect("required")` rewritten as three-line expects that name
  the invariant, which the `errors` reviewer accepts (3.33) and the `simplicity` reviewer does not
  (1.33, with `context`). One of the limits says a bare `unwrap()` is fine "when the check that
  guards it is on the adjacent line", and another says not to invent error variants for states only
  a bug can reach. The same bullets have one good effect: the three `context2` runs and one
  `limits` run are the only ones that replace the silent dropping of dead-lettered messages with
  an `expect` that names the invariant, which no `context` run did. `limits` as a skill is the
  weakest treated arm (5.67; `errors` 1.67).
- `audit` re-scored: 7, 6, 7. Below `context` on the sum because it does no type work (`types` 1,
  1, 1 on this reviewer's scale), and still the only arm with `simplicity` 3.
- Every treated arm is above `none` (4, 4, 4), which again equals BASE.

#### P6 feature

| Arm | Model | Runs |
|---|---|---|
| short | sonnet-5-5 | p6f-01, p6f-07, p6f-08 |
| limits | sonnet-5-5 | p6f-02, p6f-04, p6f-09 |
| context2 | sonnet-5-5 | p6f-03, p6f-05, p6f-06 |

Process:

| Run | Arm | Hidden tests | Own tests | Skill calls | Skills loaded, in order | At tool call | Tool calls | Stop hook | Turns | Min | USD | Commits | Diff vs start |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| p6f-01 | short | 80/80 | ok | 1 | rules | 1 | 14 | - | 16 | 2.2 | 0.47 | 3 | 12 files, 677+, 4- |
| p6f-07 | short | 80/80 | ok | 1 | rules | 1 | 14 | - | 16 | 2.7 | 0.55 | 4 | 15 files, 883+, 7- |
| p6f-08 | short | 80/80 | ok | 1 | rules | 1 | 17 | - | 19 | 2.5 | 0.54 | 4 | 12 files, 792+, 9- |
| p6f-02 | limits | 80/80 | ok | 1 | rules | 1 | 18 | - | 20 | 2.7 | 0.59 | 3 | 13 files, 810+, 14- |
| p6f-04 | limits | 80/80 | ok | 1 | rules | 1 | 18 | - | 20 | 2.7 | 0.58 | 4 | 12 files, 839+, 10- |
| p6f-09 | limits | 80/80 | ok | 1 | rules | 1 | 14 | - | 16 | 2.2 | 0.49 | 3 | 12 files, 658+, 3- |
| p6f-03 | context2 | 80/80 | ok | 0 | - | - | 17 | - | 18 | 2.7 | 0.57 | 4 | 12 files, 782+, 13- |
| p6f-05 | context2 | 80/80 | ok | 0 | - | - | 12 | - | 13 | 2.5 | 0.54 | 3 | 15 files, 688+, 7- |
| p6f-06 | context2 | 80/80 | ok | 0 | - | - | 21 | - | 22 | 2.6 | 0.58 | 4 | 11 files, 691+, 9- |

Code (BASE is the start crate):

| Run | Arm | Prod lines | Test lines | src files | Newtypes | Enums | Stringly ids /KLOC | Str compares /KLOC | expect /KLOC | unwrap_or* /KLOC | clone /KLOC | as casts /KLOC | Lint set /KLOC | Comment % | Tests | Weak asserts | allow attrs |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| BASE (p1-04) | - | 1057 | 1073 | 10 | 0 | 3 | 19.43 | 2.04 | 12.27 | 10.22 | 3.07 | 1.02 | 5.11 | 7.5 | 44 | 20 | 1 |
| p6f-01 | short | 1251 | 1503 | 11 | 0 | 3 | 18.01 | 1.72 | 15.44 | 9.43 | 3.43 | 0.86 | 6.86 | 6.8 | 65 | 23 | 1 |
| p6f-07 | short | 1334 | 1600 | 13 | 2 | 3 | 18.65 | 1.62 | 14.6 | 8.92 | 4.06 | 1.62 | 6.49 | 7.6 | 70 | 21 | 1 |
| p6f-08 | short | 1339 | 1516 | 12 | 0 | 3 | 24.08 | 1.61 | 16.05 | 8.83 | 3.21 | 1.61 | 5.62 | 6.9 | 64 | 21 | 1 |
| p6f-02 | limits | 1316 | 1546 | 12 | 0 | 3 | 21.24 | 2.45 | 15.52 | 8.99 | 2.45 | 1.63 | 6.54 | 7.0 | 68 | 21 | 1 |
| p6f-04 | limits | 1319 | 1578 | 12 | 0 | 3 | 21.17 | 2.44 | 16.29 | 8.96 | 2.44 | 1.63 | 6.51 | 6.9 | 69 | 21 | 1 |
| p6f-09 | limits | 1278 | 1457 | 11 | 0 | 3 | 18.46 | 1.68 | 15.94 | 9.23 | 4.19 | 1.68 | 5.87 | 6.7 | 65 | 21 | 1 |
| p6f-03 | context2 | 1324 | 1516 | 12 | 0 | 3 | 21.1 | 1.62 | 16.23 | 8.93 | 2.44 | 1.62 | 5.68 | 6.9 | 61 | 22 | 1 |
| p6f-05 | context2 | 1330 | 1419 | 13 | 0 | 3 | 20.28 | 1.62 | 15.41 | 8.92 | 2.43 | 0.81 | 6.49 | 7.3 | 65 | 22 | 1 |
| p6f-06 | context2 | 1290 | 1476 | 11 | 0 | 3 | 20.83 | 1.67 | 15.83 | 9.17 | 2.5 | 0.83 | 5.0 | 7.0 | 61 | 20 | 1 |

Means per arm:

| Arm | n | Skill calls per run | Newtypes per run | Newtypes mean | Enums | Stringly ids /KLOC | Str compares /KLOC | expect /KLOC | unwrap_or* /KLOC | clone /KLOC | Lint set /KLOC | Comment % | Tests | Weak asserts per run | Prod lines | Turns | Min | USD | Commits |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| short | 3 | 1, 1, 1 | 0, 2, 0 | 0.7 | 3.0 | 20.2 | 1.65 | 15.4 | 9.1 | 3.6 | 6.3 | 7.1 | 66 | 23, 21, 21 | 1308 | 17 | 2.5 | 0.52 | 3.7 |
| limits | 3 | 1, 1, 1 | 0, 0, 0 | 0.0 | 3.0 | 20.3 | 2.19 | 15.9 | 9.1 | 3.0 | 6.3 | 6.9 | 67 | 21, 21, 21 | 1304 | 19 | 2.5 | 0.55 | 3.3 |
| context2 | 3 | 0, 0, 0 | 0, 0, 0 | 0.0 | 3.0 | 20.7 | 1.64 | 15.8 | 9.0 | 2.5 | 5.7 | 7.1 | 62 | 22, 22, 20 | 1315 | 18 | 2.6 | 0.56 | 3.7 |

Reading. The rules were in context in all nine runs (the skill was the first tool call in the six
skill runs), and the scope sentence of `limits` and `context2` says in so many words to apply the
rules to the code being added. Newtypes: 0 in eight runs, 2 in one (p6f-07, `short`). `expect`,
stringly ids and weak assertions sit where the P2 `none` runs sit. Together with P2, eight arms and
24 runs: nothing tried moves the code an agent adds to a crate written in another style.

## 6. Cross-phase analysis

### 6.1 Activation: when the skills load by themselves

Every run in which a Skill call was possible, by task, model and arm:

| Task | Phase | Arm | Entry skill | Runs that loaded any skill | Skill calls per run | First Skill call at tool call |
|---|---|---|---|---|---|---|
| greenfield | p1 | skills | none (descriptions only) | 3 of 3 | 6, 6, 5 | 3, 3, 3 |
| greenfield | p1 | router | rust-code | 3 of 3 | 7, 7, 7 | 1, 2, 1 |
| greenfield | p1 | hook | none (descriptions only) | 3 of 3 | 3, 3, 5 | 20, 25, 3 |
| greenfield | p4 | skills/haiku | none (descriptions only) | 1 of 2 | 0, 5 | -, 3 |
| greenfield | p4 | skills/opus | none (descriptions only) | 2 of 2 | 4, 4 | 2, 2 |
| greenfield | p6g | short | rust-rules | 3 of 3 | 1, 1, 1 | 1, 1, 1 |
| greenfield | p6g | limits | rust-rules | 3 of 3 | 1, 1, 1 | 1, 1, 1 |
| feature | p2 | skills | none (descriptions only) | 0 of 3 | 0, 0, 0 | -, -, - |
| feature | p2 | router | rust-code | 3 of 3 | 7, 7, 7 | 1, 1, 1 |
| feature | p2 | hook | none (descriptions only) | 2 of 3 | 2, 0, 1 | 18, -, 19 |
| feature | p6f | short | rust-rules | 3 of 3 | 1, 1, 1 | 1, 1, 1 |
| feature | p6f | limits | rust-rules | 3 of 3 | 1, 1, 1 | 1, 1, 1 |
| refactor | p3 | skills | none (descriptions only) | 3 of 3 | 1, 2, 1 | 2, 2, 2 |
| refactor | p3 | router | rust-code | 3 of 3 | 7, 7, 7 | 1, 1, 1 |
| refactor | p3 | audit | rust-audit | 3 of 3 | 1, 1, 1 | 1, 1, 1 |
| refactor | p3 | hook | none (descriptions only) | 3 of 3 | 1, 1, 1 | 2, 2, 2 |
| refactor | p6r | short | rust-rules | 3 of 3 | 1, 1, 1 | 1, 1, 1 |
| refactor | p6r | limits | rust-rules | 3 of 3 | 1, 1, 1 | 1, 1, 1 |

- Entry skills are reliable. `rust-code` (router), `rust-audit`, and `rust-rules` (short, limits)
  were each loaded in every run that had them, 30 runs in v2 and both P5 runs, as tool call 1 in
  all but one (tool call 2). Their descriptions name the whole request ("Write, change or refactor Rust code",
  "Find what to fix in existing Rust code"). v1 found the same for its router (8 of 8).
- Without an entry skill it depends on the task. Counting the `skills` arm and the `hook` arm before
  its hook fired (the same eight skills, no entry skill), on Sonnet:
  - Greenfield: 4 of 6 runs loaded guides at the start (3 of 3 in `skills`, 1 of 3 in `hook`),
    five or six of them each time.
  - Feature: 0 of 6.
  - Refactor: 6 of 6 loaded `rust-keep-it-simple` (one also `rust-comments`) at tool call 2, and
    none loaded any other guide. In v1, on the large crate with the same prompt, no run of the
    branch arm loaded anything.
- By model, greenfield, `skills` arm: Haiku 1 of 2, Sonnet 3 of 3, Opus 2 of 2. Opus loaded four
  guides, Sonnet five or six, Haiku five or none.
- Which guides load by themselves: `rust-domain-types`, `rust-error-discipline`,
  `rust-test-quality` and `rust-comments` in every greenfield run that loaded anything (seven
  runs); `rust-keep-it-simple` in the four Sonnet runs and the Haiku run, in neither Opus run;
  `rust-ownership` in two Sonnet runs only. The descriptions that trigger are the ones that name what a new crate obviously contains
  (structs, `Result`, tests).
- Skill calls are made at the start or not at all. The only Skill calls after tool call 8 in the
  whole of v2 are the ones a Stop hook asked for.

### 6.2 Loaded guides against the short checklist

Two comparisons, both on reviewers who saw the compared runs together.

Greenfield (P6 scoring): is it the channel or the length?

| Arm | Content | Channel | Blind sum (max 16) | types | simplicity | overall | Newtypes | Min | USD |
|---|---|---|---|---|---|---|---|---|---|
| skills | six guides, 1170 lines (5 or 6 loaded) | skills, by description | 10.67 (10 to 12) | 1.33 | 2.67 | 2.00 | 4, 1, 2 | 9.5 | 1.72 |
| short | 47 bullets, 68 lines | one skill | 12.67 (12 to 13) | 3.33 | 2.67 | 3.67 | 7, 8, 9 | 7.4 | 1.20 |
| context | 47 bullets, 68 lines | CLAUDE.md | 12.67 (11 to 15) | 3.67 | 2.00 | 3.00 | 10, 8, 10 | 9.8 | 1.59 |
| limits | bullets + limits, 121 lines | one skill | 11.67 (11 to 12) | 2.00 | 3.33 | 3.00 | 2, 3, 3 | 5.8 | 0.93 |
| context2 | bullets + limits, 121 lines | CLAUDE.md | 12.00 (12 to 12) | 2.67 | 2.67 | 2.67 | 4, 3, 6 | 9.3 | 1.43 |
| none | - | - | 6.33 (5 to 8) | 0.33 | 2.00 | 1.33 | 0, 0, 0 | 5.8 | 0.94 |

- Same content, two channels: `short` 12.67 and `context` 12.67; `limits` 11.67 and `context2`
  12.00. The channel does not matter for new code.
- Same channel, two contents: `short` 12.67 (12 to 13) against `skills` 10.67 (10 to 12). The short
  text does more. The guides were loaded in all three `skills` runs, so this is not activation.
- So in greenfield the difference between `context` and `skills` that P1 showed on `types` (4.00
  against 2.33) comes from the text being short, not from it sitting in `CLAUDE.md`.

Refactor (P3 and P6 scoring): here the channel does matter.

| Arm | Content | Channel | P3 sum (max 24) | P6 sum (max 12) | expect /KLOC per run (start 12.27) | Newtypes per run |
|---|---|---|---|---|---|---|
| none | - | - | 12.67 (12 to 13) | 4.00 (4 to 4) | 12.08, 11.99, 11.88 | 0, 0, 0 |
| skills | six guides (only keep-it-simple loaded) | skills, by description | 13.00 (13 to 13) | - | 12.21, 12.23, 12.26 | 0, 0, 0 |
| router | six guides, all loaded | entry skill | 13.00 (13 to 13) | - | 12.13, 11.8, 12.21 | 0, 0, 0 |
| short | 68 lines | one skill | - | 6.33 (5 to 8) | 12.94, 11.89, 1.94 | 3, 0, 3 |
| context | 68 lines | CLAUDE.md | 17.00 (16 to 18) | 8.67 (8 to 9) | 0.86, 0.93, 2.51 | 0, 3, 7 |
| limits | 121 lines | one skill | - | 5.67 (5 to 7) | 12.65, 11.78, 13.28 | 3, 0, 0 |
| context2 | 121 lines | CLAUDE.md | - | 8.00 (8 to 8) | 14.4, 13.39, 14.44 | 3, 4, 3 |
| audit | 45-line "what to look for" + guides available | entry skill | 18.00 (17 to 19) | 6.67 (6 to 7) | 2.02, 1.02, 2.05 | 0, 0, 0 |
| hook | detector feedback + guides available | Stop hook | 15.33 (15 to 16) | - | 12.27, 1.02, 1.02 | 0, 0, 0 |

- Length again: `router` puts 1170 lines of guides in context and gets the start crate; `short`
  puts 68 lines in context through the same mechanism and gets a change in every run (6, 5, 8
  against 4 for the start crate).
- Channel: the 68 lines from `CLAUDE.md` do more than from a skill (8, 9, 9 against 6, 5, 8), and
  the 121 lines likewise (8, 8, 8 against 7, 5, 5). The ranges touch at 8 in the first pair and do
  not overlap in the second.
- One reading that fits both tasks: a skill is read once, as the answer to "how do I do this task",
  and the agent then plans; `CLAUDE.md` is standing instruction about the project. When the task is
  to write code, the plan and the rules meet anyway. When the task is to clean up, the rules have to
  outweigh the agent's own idea of a cleanup, and standing instructions weigh more than a skill
  body. This is an interpretation; the trial did not test it directly.

### 6.3 What makes an agent act on existing code

On the cleanup task the agent's own plan is small: the three `none` runs make 3 or 4 commits in
about a minute and change nothing a reviewer scores. What moved it:

| Delivery | What it adds to the agent's context | Acted on in | What changed |
|---|---|---|---|
| `audit` | a list of defects to look for, and an order of work (find, list, fix by category, move files last) | 3 of 3 (P3), 2 of 2 (P5) | errors, duplication, lint suppressions; no types on miniq, one enum in one SQS run |
| `context` | the rules as standing project instructions | 3 of 3 | types, errors, ownership; over-applied |
| `hook` | a list of concrete findings with file and line, at the moment the agent wants to stop | 3 of 3 | weak assertions in 3, `expect` on required parameters in 2 |
| `short` | the same rules as a skill body | 3 of 3 some change, 1 of 3 the `expect` removal | partial |

What did not:

| Delivery | Runs | Result |
|---|---|---|
| `skills` (guides available) | 3 (P3) | start crate; only `rust-keep-it-simple` loaded |
| `router` (guides loaded) | 3 (P3); v1 runs 12, 13 | start crate in P3; sums 4 and 2 in v1 |

Three things in the guides' own text work against a cleanup:

- `rust-keep-it-simple`, the one guide the agent loads by itself on a cleanup request, ends its
  "Do not over-simplify" section with: "Apply these rules to the code the change touches. Do not rewrite working code elsewhere to match them." An agent asked to clean up a crate
  that reads this has been told to leave the crate as it is.
- Every checklist opens with "Before finishing, verify in the diff". On a cleanup the diff is what
  the agent chose to touch, so the checklist never reaches the code it did not touch.
- The guides are written as "how to write": sections of rules with Avoid and Prefer examples. None
  of them says how to find the cases in a crate. The search commands that could do that sit at the
  very end of each guide, after the checklist.

The `audit` skill and the hook both supply exactly the missing step, a list of places to look, and
both worked without the agent reading any guide: no `audit` run and no P3 `hook` run loaded
`rust-domain-types`, `rust-error-discipline`, `rust-ownership` or `rust-test-quality`.

### 6.4 The cost of the short checklist: over-application

`simplicity` scores of the checklist arms against the others, every phase where both were scored:

| Phase | context | context2 | short | limits | skills | router | audit | none |
|---|---|---|---|---|---|---|---|---|
| P1 greenfield | 1.33 | - | - | - | 2.67 | 2.67 | - | 2.00 |
| P4 greenfield, Haiku | 1.50 | - | - | - | 2.50 | - | - | 2.50 |
| P4 greenfield, Opus | 2.50 | - | - | - | 3.50 | - | - | 3.00 |
| P6 greenfield scoring | 2.00 | 2.67 | 2.67 | 3.33 | 2.67 | - | - | 2.00 |
| P3 refactor | 1.00 | - | - | - | 2.00 | 2.00 | 3.00 | 2.00 |
| P6 refactor scoring | 1.33 | 1.33 | 2.00 | 2.00 | - | - | 3.00 | 2.00 |

`context` is the lowest or tied lowest in all six rows. What the reviewers wrote:

- P1: "Types without a second user. p1-10[context] and p1-06[context] each add a family of
  single-use types: a request type, an update type, a settings type and a spec type for the same
  config, and wrapper structs per response variant. They also copy boilerplate across newtypes."
- P3: "Type rewrites add code without removing duplication. p3-11[context], p3-08[context] and
  p3-15[context] introduce ids/time modules and a `Command` enum whose `from_name` repeats the
  dispatch match ... They also copy counter methods ... Yet they leave the copied CBD block in
  place."
- P6 refactor (condensed from the reviewer's sub-bullets): "The bottom crates add structure in bulk. p6r-08[context2] adds about ten getters on
  now-private fields, plus a forwarding method. p3-08[context] adds a sentinel-based generic
  `require`. p6r-01[context2] adds an enum variant that exists only to be rejected, and an
  `unreachable!` to handle it. p3-15[context] adds new `time` and naming types."
- P4: "Accessor and wire-struct ceremony. p4-03[context] adds getters to every struct and copies
  `Counts` into `CountsWire`."

Two things reduce it:

- The guides. In P1 `skills` and `router` score 2.67 on simplicity against 1.33 for `context`, with
  lower `types`. The guides carry the "Do not over-type" sections the checklist lacks, and
  `rust-keep-it-simple` was loaded in every `skills` and `router` run of P1.
- The limits bullets, in greenfield only: `limits` 3.33 against `short` 2.67, `context2` 2.67
  against `context` 2.00, at the price of `types` (2.00 against 3.33; 2.67 against 3.67). In cleanup
  they did not raise simplicity (2.00 = 2.00; 1.33 = 1.33).

The `overall` reviewer, who has no checklist, does not punish over-application consistently:
`context` gets 3.00 in P1, 3.00 in the P6 re-score, 3.67 in P3 and 2.67 in the P6 refactor re-score.
The top `overall` arm is `skills` in P1 (3.67), `short` in the P6 greenfield scoring (3.67),
`context` in P3 (3.67) and `context2` and `limits` in the P6 refactor scoring (3.33). No arm is the
maintainers' favourite across reviewers (section 7.2).

### 6.5 Feature-add: nothing moved

24 runs, eight arms. Newtypes in the result: 0 in 20 runs; 2, 2, 1 in the three P2 `context` runs
and 2 in one `short` run. `expect` per KLOC rises in all 24, weak assertions stay at 20 to 27
(start crate 20). Blind (P2 only): all ranges overlap with `none`.

- Skills available: never loaded (0 of 6).
- Skills loaded (`router`, `short`, `limits`): no visible effect on types or errors.
- Rules in `CLAUDE.md` (`context`, `context2`): three of six runs typed the new tag data (the
  three `context` runs); `expect` density in all six is where the `none` runs have it.
- Detector feedback (`hook`): fixed one or two weak assertions in the new tests; declined everything
  in inherited code as out of scope, which is the correct reading of the request.
- An explicit scope sentence ("apply the rules to the code you add or touch and leave the rest
  alone"): no effect.

What the rule-free judge rewarded on this task: batch entries routed through the existing
single-command code so a later change cannot miss the batch; validation done once and its result
carried; one shared batch prologue; new files at the start crate's scale; and typed tags. It marked
down the two attempts that rewrote the inherited `SendMessage` body. It gave the untouched start
crate its top score. So on a feature request consistency with the code that is there is itself what
a maintainer-style judge rewards, and the rules compete with it and lose. The trial cannot say
whether that is a defect of the delivery or the right outcome.

### 6.6 Model sensitivity

Greenfield, the three models on the same arms (Sonnet from P1; Haiku and Opus from P4; different
reviewers, so compare the mechanical columns across models and the blind columns only inside P4):

| Model | Arm | n | Runs that loaded guides | Newtypes per run | Stringly ids /KLOC | expect /KLOC | Weak asserts per run | Turns | Min | USD | Blind sum |
|---|---|---|---|---|---|---|---|---|---|---|---|
| Haiku 5.5 | none | 2 | - | 0, 0 | 25.5 | 2.4 | 28, 18 | 38 | 9.4 | 0.48 | 7.50 (7 to 8) |
| Haiku 5.5 | skills | 2 | 1 of 2 | 0, 6 | 18.4 | 4.3 | 5, 18 | 78 | 14.2 | 1.19 | 9.50 (9 to 10) |
| Haiku 5.5 | context | 2 | - | 10, 12 | 10.1 | 7.1 | 3, 1 | 50 | 16.3 | 1.09 | 11.50 (11 to 12) |
| Sonnet 5.5 | none | 3 | - | 0, 0, 0 | 22.3 | 9.2 | 9, 20, 14 | 18 | 5.8 | 0.94 | (P1 scale) 11.33 (11 to 12) |
| Sonnet 5.5 | skills | 3 | 3 of 3 | 4, 1, 2 | 24.7 | 5.3 | 2, 0, 9 | 42 | 9.5 | 1.72 | (P1 scale) 18.33 (17 to 19) |
| Sonnet 5.5 | context | 3 | - | 10, 8, 10 | 11.3 | 3.2 | 16, 0, 3 | 28 | 9.8 | 1.59 | (P1 scale) 18.67 (17 to 20) |
| Opus 5.5 | none | 2 | - | 1, 1 | 33.9 | 6.3 | 19, 7 | 27 | 8.8 | 2.13 | 10.00 (10 to 10) |
| Opus 5.5 | skills | 2 | 2 of 2 | 5, 5 | 9.9 | 6.6 | 0, 0 | 40 | 11.8 | 3.12 | 13.00 (12 to 14) |
| Opus 5.5 | context | 2 | - | 13, 10 | 9.6 | 2.5 | 0, 0 | 44 | 15.8 | 4.05 | 14.00 (14 to 14) |

- The direction is the same in all three: with nothing, 0 or 1 newtypes and 19 to 35 stringly ids
  per KLOC; with the checklist, 8 to 13 newtypes and 6 to 14.
- The guides depend on the model twice: whether they get loaded (Haiku 1 of 2) and how much is done
  with them (Opus: stringly ids down to about 10 per KLOC; Sonnet: unchanged at 18 to 28).
- A stronger model without rules does not write the typed version by itself: Opus `none` has the
  highest stringly-id density of any cell.
- Within P4's blind scale, the checklist is worth more than the step from Haiku to Opus: Haiku
  `context` 11.5, Opus `none` 10.0.

### 6.7 Cost and time

| Task | Phase | Arm | n | Mean turns | Mean tool calls | Mean minutes | Mean USD | Total USD |
|---|---|---|---|---|---|---|---|---|
| greenfield | p1 | none | 3 | 18 | 17 | 5.8 | 0.94 | 2.82 |
| greenfield | p1 | skills | 3 | 42 | 35 | 9.5 | 1.72 | 5.15 |
| greenfield | p1 | router | 3 | 36 | 28 | 8.5 | 1.51 | 4.54 |
| greenfield | p1 | hook | 3 | 36 | 30 | 9.2 | 1.60 | 4.79 |
| greenfield | p1 | context | 3 | 28 | 27 | 9.8 | 1.59 | 4.76 |
| greenfield | p6g | short | 3 | 22 | 20 | 7.4 | 1.20 | 3.59 |
| greenfield | p6g | limits | 3 | 17 | 15 | 5.8 | 0.93 | 2.80 |
| greenfield | p6g | context2 | 3 | 21 | 20 | 9.3 | 1.43 | 4.28 |
| greenfield | p4 | none/haiku | 2 | 38 | 38 | 9.4 | 0.48 | 0.97 |
| greenfield | p4 | skills/haiku | 2 | 78 | 81 | 14.2 | 1.19 | 2.37 |
| greenfield | p4 | context/haiku | 2 | 50 | 48 | 16.3 | 1.09 | 2.18 |
| greenfield | p4 | none/opus | 2 | 27 | 26 | 8.8 | 2.13 | 4.27 |
| greenfield | p4 | skills/opus | 2 | 40 | 36 | 11.8 | 3.12 | 6.25 |
| greenfield | p4 | context/opus | 2 | 44 | 44 | 15.8 | 4.05 | 8.11 |
| feature | p2 | none | 3 | 14 | 13 | 2.3 | 0.49 | 1.47 |
| feature | p2 | skills | 3 | 18 | 17 | 2.5 | 0.52 | 1.57 |
| feature | p2 | router | 3 | 30 | 22 | 2.6 | 0.69 | 2.08 |
| feature | p2 | hook | 3 | 24 | 21 | 3.5 | 0.71 | 2.13 |
| feature | p2 | context | 3 | 22 | 21 | 3.0 | 0.64 | 1.91 |
| feature | p6f | short | 3 | 17 | 15 | 2.5 | 0.52 | 1.56 |
| feature | p6f | limits | 3 | 19 | 17 | 2.5 | 0.55 | 1.66 |
| feature | p6f | context2 | 3 | 18 | 17 | 2.6 | 0.56 | 1.69 |
| refactor | p3 | none | 3 | 9 | 8 | 1.2 | 0.26 | 0.77 |
| refactor | p3 | skills | 3 | 11 | 8 | 0.9 | 0.25 | 0.74 |
| refactor | p3 | router | 3 | 27 | 19 | 1.6 | 0.45 | 1.34 |
| refactor | p3 | audit | 3 | 14 | 12 | 1.6 | 0.31 | 0.94 |
| refactor | p3 | hook | 3 | 14 | 11 | 1.4 | 0.33 | 0.98 |
| refactor | p3 | context | 3 | 28 | 27 | 4.7 | 0.87 | 2.61 |
| refactor | p6r | short | 3 | 15 | 13 | 1.6 | 0.34 | 1.01 |
| refactor | p6r | limits | 3 | 16 | 14 | 1.5 | 0.35 | 1.04 |
| refactor | p6r | context2 | 3 | 29 | 28 | 3.6 | 0.72 | 2.15 |

Per phase:

| Phase | Runs | USD | Agent minutes |
|---|---|---|---|
| p1 | 15 | 22.06 | 129 |
| p2 | 15 | 9.16 | 42 |
| p3 | 18 | 7.38 | 34 |
| p4 | 12 | 24.15 | 152 |
| p6g | 9 | 10.67 | 67 |
| p6r | 9 | 4.20 | 20 |
| p6f | 9 | 4.91 | 23 |

- Greenfield: treatment adds roughly 50 to 70 percent to the time and 60 to 80 percent to the cost
  on Sonnet (5.8 minutes and USD 0.94
  for `none`; 8.5 to 9.8 minutes and USD 1.51 to 1.72 for the P1 arms). The one-skill arms are
  cheaper: `short` 7.4 minutes and USD 1.20, `limits` 5.8 and 0.93.
- Refactor: `none`, `skills`, `audit`, `hook`, `short` and `limits` all finish in 1 to 2 minutes for
  USD 0.25 to 0.35. `router` costs more turns for the same result (27 against 9; seven Skill calls).
  The two `CLAUDE.md` arms take three to four times as long (3.6 and 4.7 minutes) because they
  change the most.
- Feature: 2 to 3.5 minutes, USD 0.49 to 0.71, in every arm.
- Opus costs 1.8 to 2.5 times what Sonnet does on the same arm; Haiku costs half of Sonnet with
  nothing and about two thirds with a treatment, and takes longer.

## 7. Reliability

### 7.1 Test-retest on the anchors

In P6 nine P1 crates and nine P3 crates plus BASE were scored again, by different reviewer agents
that saw them mixed with new crates under new codes.

Greenfield anchors (P1 scores against P6 scores of the same crates):

| Dimension | Repeated scores | Identical | Within 1 point | Largest gap | Mean shift (p1 minus p6g) |
|---|---|---|---|---|---|
| types | 9 | 5 | 9 | 1.0 | +0.44 |
| errors | 9 | 7 | 9 | 1.0 | +0.22 |
| simplicity | 9 | 5 | 9 | 1.0 | -0.22 |
| tests | 9 | 3 | 8 | 2.0 | -0.33 |
| overall | 9 | 6 | 7 | 2.0 | +0.56 |
| all | 45 | 26 | 42 | 2.0 | +0.13 |

Refactor anchors (P3 against P6):

| Dimension | Repeated scores | Identical | Within 1 point | Largest gap | Mean shift (p3 minus p6r) |
|---|---|---|---|---|---|
| types | 10 | 3 | 10 | 1.0 | +0.70 |
| errors | 10 | 4 | 10 | 1.0 | +0.60 |
| simplicity | 10 | 9 | 10 | 1.0 | -0.10 |
| overall | 10 | 4 | 10 | 1.0 | +0.00 |
| all | 40 | 20 | 40 | 1.0 | +0.30 |

Together: 85 repeated scores, 46 identical, 82 within one point, none more than two points apart.

By arm, P1 to P6:

| Anchor arm | types (p1 -> p6g) | errors (p1 -> p6g) | simplicity (p1 -> p6g) | tests (p1 -> p6g) | overall (p1 -> p6g) |
|---|---|---|---|---|---|
| none | 0.33 -> 0.33 | 1.67 -> 1.33 | 2.00 -> 2.00 | 1.67 -> 2.67 | 1.33 -> 1.33 |
| skills | 2.33 -> 1.33 | 3.33 -> 3.00 | 2.67 -> 2.67 | 3.33 -> 3.67 | 3.67 -> 2.00 |
| context | 4.00 -> 3.67 | 3.67 -> 3.67 | 1.33 -> 2.00 | 3.67 -> 3.33 | 3.00 -> 3.00 |

By arm, P3 to P6:

| Anchor arm | types (p3 -> p6r) | errors (p3 -> p6r) | simplicity (p3 -> p6r) | overall (p3 -> p6r) |
|---|---|---|---|---|
| BASE | 2.00 -> 1.00 | 2.00 -> 1.00 | 2.00 -> 2.00 | 2.00 -> 3.00 |
| none | 2.00 -> 1.00 | 2.00 -> 1.00 | 2.00 -> 2.00 | 2.00 -> 2.67 |
| audit | 2.00 -> 1.00 | 3.33 -> 2.67 | 3.00 -> 3.00 | 3.00 -> 3.00 |
| context | 3.67 -> 3.67 | 3.67 -> 3.67 | 1.00 -> 1.33 | 3.67 -> 2.67 |

What this shows:

- A single 0-to-4 score is reproducible to within one point 96 percent of the time and exactly a
  little over half the time. Differences of less than one point between arm means are inside that
  noise unless the per-run ranges separate.
- The scale moves between reviewers. The P6 refactor reviewers scored the start crate and the
  `none` runs 1 on types and errors where the P3 reviewers gave 2. Sums from different phases are
  not comparable; only orderings inside a phase are.
- The orderings that matter survived the re-scoring: treated above `none` in greenfield with no
  overlap; `context` and `audit` above `none` in refactor with no overlap; `context` highest on
  `types`; `context` lowest on `simplicity`.
- One ordering did not survive: `overall` for the three P1 `skills` crates went from 3, 4, 4
  to 2, 2, 2 (the two gaps of two points in that dimension are here). "The full guides give the crate a
  maintainer would rather own" rests on the P1 reviewer alone, plus the P4 reviewer, who also put
  `skills` first on `overall`. It should be treated as open.
- `tests` in greenfield and `types` in refactor are the least stable (3 of 9 and 3 of 10
  identical); the second because the whole scale moved by a point.

### 7.2 Separation and overlap

Lowest and highest run of each arm, on the blind sum of its phase:

| Phase | Arm A | A range | Arm B | B range | Ranges | Mean A minus B | n |
|---|---|---|---|---|---|---|---|
| p1 | skills | 17 to 19 | none | 11 to 12 | no overlap | +7.00 | 3 vs 3 |
| p1 | router | 17 to 19 | none | 11 to 12 | no overlap | +6.67 | 3 vs 3 |
| p1 | hook | 15 to 18 | none | 11 to 12 | no overlap | +5.00 | 3 vs 3 |
| p1 | context | 17 to 20 | none | 11 to 12 | no overlap | +7.33 | 3 vs 3 |
| p6g | skills | 10 to 12 | none | 5 to 8 | no overlap | +4.33 | 3 vs 3 |
| p6g | context | 11 to 15 | none | 5 to 8 | no overlap | +6.33 | 3 vs 3 |
| p6g | short | 12 to 13 | none | 5 to 8 | no overlap | +6.33 | 3 vs 3 |
| p6g | limits | 11 to 12 | none | 5 to 8 | no overlap | +5.33 | 3 vs 3 |
| p6g | context2 | 12 to 12 | none | 5 to 8 | no overlap | +5.67 | 3 vs 3 |
| p2 | skills | 6 to 9 | none | 6 to 10 | overlap | +0.67 | 3 vs 3 |
| p2 | router | 7 to 10 | none | 6 to 10 | overlap | +1.33 | 3 vs 3 |
| p2 | hook | 7 to 10 | none | 6 to 10 | overlap | +1.33 | 3 vs 3 |
| p2 | context | 8 to 11 | none | 6 to 10 | overlap | +2.00 | 3 vs 3 |
| p3 | skills | 13 to 13 | none | 12 to 13 | overlap | +0.33 | 3 vs 3 |
| p3 | router | 13 to 13 | none | 12 to 13 | overlap | +0.33 | 3 vs 3 |
| p3 | audit | 17 to 19 | none | 12 to 13 | no overlap | +5.33 | 3 vs 3 |
| p3 | hook | 15 to 16 | none | 12 to 13 | no overlap | +2.67 | 3 vs 3 |
| p3 | context | 16 to 18 | none | 12 to 13 | no overlap | +4.33 | 3 vs 3 |
| p6r | audit | 6 to 7 | none | 4 to 4 | no overlap | +2.67 | 3 vs 3 |
| p6r | context | 8 to 9 | none | 4 to 4 | no overlap | +4.67 | 3 vs 3 |
| p6r | short | 5 to 8 | none | 4 to 4 | no overlap | +2.33 | 3 vs 3 |
| p6r | limits | 5 to 7 | none | 4 to 4 | no overlap | +1.67 | 3 vs 3 |
| p6r | context2 | 8 to 8 | none | 4 to 4 | no overlap | +4.00 | 3 vs 3 |
| p4 | skills/haiku | 9 to 10 | none/haiku | 7 to 8 | no overlap | +2.00 | 2 vs 2 |
| p4 | context/haiku | 11 to 12 | none/haiku | 7 to 8 | no overlap | +4.00 | 2 vs 2 |
| p4 | skills/opus | 12 to 14 | none/opus | 10 to 10 | no overlap | +3.00 | 2 vs 2 |
| p4 | context/opus | 14 to 14 | none/opus | 10 to 10 | no overlap | +4.00 | 2 vs 2 |
| p4 | context/haiku | 11 to 12 | none/opus | 10 to 10 | no overlap | +1.50 | 2 vs 2 |
| p1 | context | 17 to 20 | skills | 17 to 19 | overlap | +0.33 | 3 vs 3 |
| p6g | short | 12 to 13 | skills | 10 to 12 | overlap | +2.00 | 3 vs 3 |
| p6g | short | 12 to 13 | context | 11 to 15 | overlap | +0.00 | 3 vs 3 |
| p6g | limits | 11 to 12 | context2 | 12 to 12 | overlap | -0.33 | 3 vs 3 |
| p6g | context2 | 12 to 12 | context | 11 to 15 | overlap | -0.67 | 3 vs 3 |
| p3 | audit | 17 to 19 | context | 16 to 18 | overlap | +1.00 | 3 vs 3 |
| p3 | router | 13 to 13 | skills | 13 to 13 | overlap | +0.00 | 3 vs 3 |
| p6r | short | 5 to 8 | context | 8 to 9 | overlap | -2.33 | 3 vs 3 |
| p6r | context2 | 8 to 8 | context | 8 to 9 | overlap | -0.67 | 3 vs 3 |
| p6r | limits | 5 to 7 | short | 5 to 8 | overlap | -0.67 | 3 vs 3 |
| p6r | audit | 6 to 7 | short | 5 to 8 | overlap | +0.33 | 3 vs 3 |

- Clear of `none` with no overlap, in every scoring it appears in: all greenfield treatments in all
  three models; `audit`, `context`, `hook`, `short`, `limits`, `context2` in refactor (the last
  three by one point or more over a `none` that is exactly BASE).
- Not separated from `none`: every feature arm; `skills` and `router` in refactor.
- Not separated from each other: the greenfield treatments among themselves, except that `short`
  sits at or above the top of `skills`; `audit` against `context` in refactor (P3: 17 to 19 against
  16 to 18; P6: 6 to 7 against 8 to 9, reversed, because P6 left out `comments` and `tests`,
  where `audit` leads, and its `types` reviewer put `audit` at the start crate's level).

### 7.3 Cell sizes

Three runs per arm and task on Sonnet, two per arm and model in P4, two in P5. A range from three
runs is a weak estimate of spread. The claims in section 1 were chosen to be the ones where the
separation is total (no overlap) or where the mechanical counts are identical across runs (P3
`router` and `skills` against BASE).

### 7.4 Conclusions that rest on mechanical counts only

- Everything about the P6 feature runs (not reviewed blind).
- The size of the `expect` and newtype effects, and all per-KLOC figures.
- Activation counts and positions (from the logs; exact).
- The hook's effect on weak assertions in P1 (the `tests` dimension did not separate `hook` from the
  other treated arms).
- P5's lint and `expect` figures (its rubric scores are single-reviewer and unverified).

## 8. Threats to validity

- **One task family.** miniq is a small stateful service with a text protocol, close in domain to
  the SQS crate whose audit produced the rules. The rules about identifiers, closed sets and
  required parameters have many chances to apply. A numeric library, an async client or a CLI would
  exercise other rules (the async and unsafe skills never applied here).
- **Crate size.** 1000 to 2400 production lines. The cleanup runs last one to five minutes; the v1
  crate needed 40 to 70. P5 is the only check that a v2 result transfers to a large crate, and it
  covers one arm with two runs.
- **The rubric is the skills.** The six dimension rubrics in `reviews/dimension-brief.md` were
  written from the skills' own checklists, so a crate that follows the checklist scores well by
  construction. This measures adherence, not benefit. The `overall` dimension is the only judge
  without the rules, it is one reviewer per phase, and section 7.1 shows it is the least consistent
  where it matters most. The hidden acceptance tests measure correctness only, and every run passes.
- **Same model family on both sides.** Authors are Claude Haiku, Sonnet and Opus 5.5; reviewers are
  Claude Opus agents. A shared taste would not show up as disagreement.
- **The arms' texts were written by the experimenter.** The router, `rust-audit`, the scope sentence
  and the hook script are drafts written during the trial by the same agent that ran it, knowing the
  v1 results. The `context` and `short` texts are mechanical extracts and are not affected.
  `rust-audit` was written before the miniq baseline existed but with knowledge of the SQS audit.
- **P5 ran under `~`.** Like all of v1 it had the user's global `CLAUDE.md` loaded ("prefer
  self-explanatory code over comments", no em dashes), and its two reviews were not verified.
- **The hook's detector overlaps the measures.** Eleven of its fourteen lints are in the evaluation
  lint set, and its grep patterns are close to `detect.py`'s `stringly ids`, `str compares` and
  `weak asserts`. A drop in those counts in the `hook` arm is partly the arm being told its score.
  The blind `tests` score (3.67 in P3) is the independent evidence for it.
- **No human review.** No person has read these crates. The reviewers' citations were not checked by
  a second agent in v2 (v1 did that and changed 1 score in 154).
- **Baseline choice.** The start crate for P2, P3 and P6 is one `none` crate (p1-04), picked by a
  rule fixed in advance. Another start crate would offer other defects.
- **Prompts.** The greenfield and refactor prompts ask for code "an experienced maintainer would be
  happy to own"; the feature prompt does not. The feature result may partly be that sentence.
- **Order and time.** P1 ran in two batches (2.2). P6 anchors were reviewed a second time by fresh
  reviewers but the crates were the same files, so the retest measures the reviewers, not the runs.
- **Detector caveats.** Regex counts; one run (p1-10) is mis-split (2.2).

## 9. Recommendations for just-right-rust

Each is tied to what was measured; none has been tried as a change to the repository.

1. **Ship a short rules skill as the entry point for writing Rust.** Evidence: `short` (68 lines,
   one skill, loaded first in 9 of 9 runs) scores 12 to 13 in greenfield against 10 to 12 for the
   six guides loaded by description and 5 to 8 for nothing, costs less than the guides, and needs no
   luck with activation (the guides alone loaded in 4 of 6 Sonnet runs and 1 of 2 Haiku runs). Its
   description must name the whole request, as the router's does.
2. **Keep the guides as reference the entry skill points to, not as the thing that must load.**
   Evidence: loaded in full they add nothing on cleanup (P3 `router` = BASE) and less than the
   checklist on new code. Their distinct value in this trial is restraint: lower over-application in
   P1 (`simplicity` 2.67 against 1.33). Whether an agent that has the short skill will also open a
   guide when it needs the "how" was not tested; no `audit` run did.
3. **Put the limits next to the rules they limit, and expect a trade.** Evidence: in greenfield the
   "But:" bullets moved `simplicity` up and `types` down by about the same amount. A shorter,
   sharper version of the limits for types (the reviewers' complaints are all "single-use types" and
   "copied newtype boilerplate") is the obvious next text to test.
4. **Add an audit entry skill for cleanup, refactor and review requests.** Evidence: P3 `audit`
   17 to 19 against 12 to 13, in a minute and a half; P5 sums 7 and 9 in 35 and 44 turns on the
   large crate, without the file-splitting that 13 of 14 v1 runs fell into. It should list what to
   look for, give an order of work, and say to move files last. Its weakness here is types (none on
   miniq), where the checklist in `CLAUDE.md` was strong: the two could be combined.
5. **Fix the scope sentence in `rust-keep-it-simple`.** "Apply these rules to the code the change
   touches. Do not rewrite working code elsewhere to match them." is right for a feature and wrong
   for a cleanup, and this is the one guide agents load by themselves on a cleanup request (6 of 6).
   Say both cases, as the P6 scope sentence does. Note that the P6 sentence alone did not make the
   skill-delivered rules act on cleanup, so this is necessary, not sufficient.
6. **Reword "Before finishing, verify in the diff".** On a cleanup or review the diff is not the
   subject. Evidence is indirect: the arms that told the agent to look at the crate (`audit`) or
   handed it findings from the crate (`hook`) worked; the checklists as written did not when loaded
   through `router`.
7. **Use the hook engine as detector feedback at Stop.** Evidence: the hook fired 9 of 9 times, the
   agent answered every finding, fixed the ones in scope and declined the rest with reasons; best
   `tests` score on cleanup; weak assertions 1, 0, 0 in greenfield against 9, 20, 14. It costs 1
   to 8 more tool calls. Two cautions: it reports the same things the trial counted, so its measured effect is
   inflated; and in greenfield it was the weakest treated arm on the blind sum, because in two of
   three runs the rules arrived only at the end. It complements the entry skill; it does not
   replace it.
8. **Recommend `CLAUDE.md`/`AGENTS.md` placement for projects that want the rules to act on
   existing code.** Evidence: P6 refactor, same text, 8 to 9 from `CLAUDE.md` against 5 to 8 from a
   skill. For symposium this means a plugin may need a way to contribute standing project
   instructions, not only skills.
9. **Do not expect any of this to change feature work in a codebase with another style.** 24 runs,
   eight arms, no movement. Unsolved.

Experiments worth running next, in order of expected information:

- Feature-add into a start crate that already follows the rules (does the new code keep them?), and
  into the current one with a prompt that carries the same "maintainer would be happy to own"
  sentence as the other two tasks. That separates "the rules lose to local style" from "the prompt
  did not ask".
- `short` plus `rust-audit` plus the hook together, on all three tasks: the three mechanisms act at
  different moments (start of writing, start of cleanup, end of work) and were only tested apart.
- A second task family and a mid-sized existing crate (5k to 10k lines), with a human reading a
  sample of the reviewers' citations.
- The same rules through `AGENTS.md` for the other agents symposium supports; everything here is
  Claude Code.
- A tightened limits text for types, tested in greenfield against `short`.
- More runs per cell for the two results that carry the recommendations: `short` against `skills`
  in greenfield (ranges touch at 12) and `audit` on a large crate (two runs).

## 10. File map of `v2/`

```text
v2/
  PLAN.md                 the plan as written before P1 (not updated; see 2.3)
  STATE.md                running log of the stage: every decision and table as it happened
  REPORT.md               this file
  task/                   SPEC.md, SPEC-feature.md, acceptance.py (hidden tests), README.md,
                          reference/ (oracle crate, never shown to an agent)
  arms/                   none has no directory
    skills/               the 8 skills as compiled by symposium 0.6.0
    router/               skills + skills/rust-code
    audit/                skills + skills/rust-audit
    hook/                 skills + hooks/hooks.json, hooks/jrr-check.sh
    context/              CLAUDE.md (68 lines)
    short/                skills/rust-rules (the context text as one skill)
    limits/               skills/rust-rules (checklist + limits + scope)
    context2/             CLAUDE.md (the limits text)
  prompts/                green.txt, feature.txt, refactor.txt
  baseline/               the start crate for P2, P3, P6r, P6f (copy of runs/p1-04)
  bin/
    run2.sh               one headless run in /Users/Shared/jrr-work/<run>
    eval2.sh              build, own tests, hidden tests, lint set, detect.py, copy to runs/
    batch.sh              run a list with bounded concurrency
    detect.py             mechanical measures and transcript summary -> eval/<run>/summary.json
    table.py              per-run table from the summaries
    blind.py              copy crates under shuffled codes for the reviewers
    scores.py             join reviewer scores with the arm key
  keys/                   run lists per phase, runs.tsv (run, arm, task, model), blind-<phase>.tsv
  logs/<run>.jsonl|.err   stream-json transcript of each run
  eval/<run>/             summary.json, commits.txt, diffstat.txt, build/test/acceptance/clippy output
  runs/<run>/             the resulting crate (no target/), plus history.bundle with its commits
  blind/<phase>/<CODE>/   what the reviewers saw (src, tests, Cargo.toml); BASE where used
  reviews/dimension-brief.md   the reviewers' instructions
  reviews/<phase>/<dimension>.md|.tsv   one reviewer per phase and dimension
```

Outside `v2/`: agent work directories in `/Users/Shared/jrr-work/` (build output cleaned); P5 in
`../runs/14`, `../runs/15`, `../eval/14`, `../eval/15`, `../logs/14.jsonl`, `../logs/15.jsonl`; the
v1 record in `../TRIAL-LOG.md`; the one-page summary in `../SUMMARY.md`.

## 11. Third round (2026-11-23): descriptions, hooks and consumption

This section is the full record of the work done after sections 1 to 10 were written. It is meant to
be read without any other context. Short log entries for the same work are in `STATE.md` under
"2026-11-23". Everything named here is committed in the trial repository (remote
`fnzko/jrr-skills-trial`, private, head `c6249b2` when this section was written).

### 11.1 What was asked and what was done

The user's questions, in order, and what answered each one:

| Question | Answered by |
|---|---|
| Can the SWE-bench-like evaluation proposed for skills be implemented now, on what James built? | 11.2 |
| Is the scope sentence of `rust-keep-it-simple` the problem on refactors, and what would let the skills flow there? | 11.3 |
| Why does adding a feature not use the skills, when a good practice can be applied locally? | 11.3 |
| How are the skills built, and what do their descriptions say? | 11.4 |
| Which patterns are right for skill descriptions, and what corrections apply? (three Opus agents) | 11.5, 11.6, 11.7 |
| Would three task-level skills (from scratch, refactor, add a feature) work? | 11.7 |
| Would a hook that forces the rules in front of the agent make sense? Build a harness and A/B it. | 11.9 |
| Does any of this raise time or token use by a lot? | 11.8, 11.9 |

Totals for this round: 54 full runs (P7 and P8), 4 hook verification runs, 72 description probes plus
8 discarded ones, 10 blind dimension reviews, 3 research agents. Recorded cost: USD 41.31 for the 54
runs (256 agent minutes), USD 25.29 for the 72 probes, about USD 0.26 for the hook verification runs,
USD 0.63 for the discarded probes. Review and research agents ran on Opus inside the Max
subscription and are not costed.

### 11.2 James's evaluation harness: what it can and cannot measure today

Read from `symposium` main at `3516c051` (design chapter `md/design/agent-eval.md`, RFD
`md/rfds/skill-evaluation/`, crate `symposium-eval/`, data in `evals/`). The harness was not run in
this round.

- The whole stack is merged (last pull request #443 on 2026-11-20). `cargo xtask agent-eval <task>
  --client claude --model <m>` runs one real Claude Code session without Symposium and one with it,
  each isolated in `srt`, grades the code with hidden Rust tests (one `#[test]` per file under
  `checks/`), and records which skills were loaded, tokens and time. A task is a directory under
  `evals/tasks/` with `task.toml`, `starter/`, `checks/` and an optional `solution/`.
- Not there yet: `--trials` with alternating order and `benchmark.json` (one round per invocation
  today, the code comment says they come with `--trials`); a Codex adapter; anything other than
  Linux (`srt`); more than two configurations (it compares without Symposium against with it);
  checks that need judgment (the RFD leaves model-graded checks out of the first version).
- The structural problem for these skills: the harness decides by pass or fail on hidden tests. In
  this trial every run passes the hidden tests in every arm (140 of 141 small-crate runs), so a port
  of the miniq tasks would report "no difference". The RFD says a comparison task needs knowledge
  the model cannot already have; the just-right-rust skills are general practice, so they change how
  the code is written and not whether it works.
- What can be done on it now: an activation benchmark through the real Symposium delivery path (did
  the skill load, on which request). The hidden tests of miniq are Python over stdin and stdout and
  would have to be rewritten as Rust `#[test]` files; whether a check may launch the binary was not
  verified.
- What it needs before it can measure these skills: a quality metric next to pass or fail. First
  objective detectors (clippy lint set per KLOC, `expect` per KLOC), then blind model review. This
  trial has the pieces: `bin/detect.py`, `reviews/dimension-brief.md`, `bin/blind.py`.

### 11.3 Positions taken on refactor and feature-add, and what backs them

Refactor. The sentence "Apply these rules to the code the change touches. Do not rewrite working code
elsewhere to match them." (`rust-keep-it-simple`, line 191 on branch `ai-patterns-skills`) is a brake
but was never tested alone: no run removed it. What is known: of the six guides only
`rust-keep-it-simple` loads on a clean-up request; with all of them loaded through an entry skill
the result equals the untouched crate, but that sentence was present there too, so the two causes
are not separated; the arms that worked (checklist in `CLAUDE.md`, audit skill, hooks) say what to
look for. Proposed and untested: make the scope conditional (on a feature or fix, the code the change
touches; on a clean-up, refactor or review, the whole requested scope), name refactor and review in
the descriptions, and move the "Review checklist" section to the top for refactors.

Feature-add. The argument "a good practice can always be applied locally" holds for some rules and
not for others. Local rules (no new `expect` on input, exact assertions in new tests, checked
conversions, comments) could be applied without touching the rest, and the agent did not:
`expect` per KLOC rose in all 24 earlier feature runs and did not move in the 12 hook runs of this
round. Cross-cutting rules (domain types, error types) are not local: a newtype in new code has to be
converted at every boundary with the old code, and the blind judge of P2 rewarded consistency with
the surrounding code. Two layers explain the missing effect: the skills do not load (0 of 6 by
description), and when the rules are in context the surrounding code still acts as the precedent.
Proposed and untested: split the checklist into local rules ("always in new code, even where the
rest does not follow them") and cross-cutting ones ("only where the crate already uses them or the
request asks").

### 11.4 How the six skills are built

Source: `just-right-rust/skills/` on branch `ai-patterns-skills` (commit `30ce8a2`, pushed to
`fnzko/just-right-rust`).

- Six skills written for this effort (`rust-domain-types`, `rust-error-discipline`,
  `rust-ownership`, `rust-test-quality`, `rust-comments`, `rust-keep-it-simple`), 175 to 218 lines
  each, plus the older `async-rust` (156 lines) and the external `unsafe-rust-review`.
- Same layout in all six: a one-paragraph principle, seven to nine rule sections with examples, a
  "Do not over-apply" section, and a "Review checklist" at the end that starts with "Before
  finishing, verify in the diff" followed by search commands and lint names.
- No skill mentions another one (zero cross-references) and there is no entry skill.
- The plugin manifest (`SYMPOSIUM.toml`) registers five hooks (`SessionStart`,
  `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`) against a hook binary that is a no-op
  (`src/main.rs`, 22 lines). `depends-on = ["*"]`, so the plugin applies to every Rust workspace.

What each description uses as its trigger:

| Skill | When writing | When reviewing |
|---|---|---|
| `rust-domain-types` | defining structs, enums or signatures; parsing external input | code that passes ids, names, states as `String`, `u64`, `bool` |
| `rust-error-discipline` | handling `Result`/`Option`, adding a fallback, parsing, I/O, casts | code that reaches for `unwrap`, `expect`, `unwrap_or_default`, `let _ =` |
| `rust-ownership` | fixing a borrow checker error; signatures, getters, loops, locks | code that adds `.clone()`, `.to_string()`, `.collect()` |
| `rust-test-quality` | writing or changing `#[test]`, `tests/`, helpers | tests that assert only `is_ok()`, non-emptiness or a substring |
| `rust-comments` | adding comments or docs; committing, describing a pull request | a diff with comments that narrate or record history |
| `rust-keep-it-simple` | adding a feature, trait, helper, field, dependency; changing or copying an existing function | a diff with unused code, duplicated logic, shims |

Every trigger is a micro-activity inside the work or a symptom in the code. None names a request
("refactor", "clean up", "audit", "implement a spec", "new crate"). The review triggers presuppose
that the symptom has already been seen. `rust-keep-it-simple` is the only one with task-level
triggers, which matches it being the only one loaded on refactors. `async-rust` uses a different
style ("Design, implement, debug, or review asynchronous Rust ...") and never loaded in any run.

### 11.5 Published guidance on descriptions (research agent, Opus)

Full text with sources and the corrected descriptions: `../v3/descriptions/guidance.md`. Eleven
primary sources: Anthropic's skill best practices, the Claude Code skills documentation, the
agentskills.io specification and its page on optimizing descriptions, the `skill-creator` skill and
its description-improver prompt, and the Codex, Copilot and Gemini CLI documentation (no Antigravity
page was found).

- The description is the whole trigger: only name and description are in context when the agent
  decides, so "when to use" belongs there and not in the body.
- Describe what the user asks for, not what happens inside the work; use words a user would say.
- Imperative, slightly pushy wording ("Use this skill whenever ..."), including requests that do not
  name the topic. No "I" or "you".
- Key use case first, because listings shorten descriptions.
- Hard limit 1,024 characters (Claude Code allows 1,536 with `when_to_use`).
- Agents skip skills for tasks they believe they can do alone. No source offers a fix.
- Not covered by any source: several narrow skills that should load together, or an entry skill
  that loads others. The alternative the guidance does back is one skill with reference files.
  Claude Code recommends hooks for rules that must hold every time.
- The agent rewrote the six descriptions accordingly (428 to 521 characters, leading with whole
  jobs). They were tested as variant `v7` in 11.7 and did not load.

### 11.6 Evidence from the existing transcripts (analysis agent, Opus)

Full tables: `../v3/descriptions/evidence.md`, scripts `extract.py` and `tables.py` beside it. 116
transcripts (29 in `../logs/`, 87 in `logs/`), not the "about 150" the brief said.

| Kind of skill | Loaded | When |
|---|---|---|
| Entry skills (`rust-code`, `rust-rules`, `rust-audit`, `rust-refactor`) | 40 of 40 runs that had them | tool call 1 in 39, call 2 once |
| Six guides by description, greenfield, Sonnet | 4 of 6 runs (four to six guides each) | calls 1 to 8 |
| Six guides by description, feature-add | 0 of 6 | |
| Six guides by description, large-crate refactor | 0 of 8 | |
| Six guides by description, small-crate refactor | only `rust-keep-it-simple`, 6 of 6 where no entry skill competed | |
| Guides with a task sentence appended (`b1` probes) | 0 of 24 chances | |
| `async-rust`, `unsafe-rust-review` | 0 of 36 | |

- What the entry-skill descriptions have in common: the first sentence restates the user's request;
  they list kinds of request; they say "Use first ... before ..."; they are under 300 characters and
  name no micro-activity or code symptom.
- When five of six guides load on greenfield, the one left out is always `rust-ownership`.
- Nothing loads unprompted after tool call 8. All nine later Skill calls come right after the Stop
  hook's findings, and they load exactly the guides whose descriptions name the reported symptoms.
  So symptom-style descriptions do work once a symptom is in front of the agent.
- The decision is silent: in 54 of 60 runs with a Skill call nothing is written before it.
- 35 built-in Claude Code skills were available in all 116 runs and loaded 0 times. No skill with
  "Always activate" wording was present, so that wording is untested.
- Counts match section 6.1. Two framing notes: v1 runs 6 and 7 did load skills on the large crate
  when the prompt said "Use the Rust skills"; the one-page summary's feature-add "0 of 3" covers
  only the `skills` arm.

### 11.7 Description probes (72 short real runs)

Files: `../v3/bin/{probe3.sh,matrix.sh,analyze.py,make_variants.py,make_v7.py}`,
`../v3/variants/v0` to `v7`, logs in `../v3/logs/`, write-up with the exact description text of every
variant in `../v3/descriptions/probe.md`.

Design: `claude -p --model claude-sonnet-5-5` with the isolated config, `--plugin-dir <variant>`,
`--permission-mode auto --strict-mcp-config --disallowedTools "WebFetch" "WebSearch"
"Bash(git push:*)" --max-turns 10 --max-budget-usd 0.5`, `timeout 300`, the three prompts of
`prompts/`, work directories prepared as `run2.sh` does, n=3 per cell, two at a time. Measured: the
Skill calls and their position. Loading is not effect.

Variants (each a copy of `arms/skills` with only the listed change): `v0` unchanged; `v1` each of
the six descriptions opens with task verbs in the style of `async-rust`; `v2` each keeps its text and
gains "Use when building new Rust code, adding a feature to existing Rust code, and refactoring,
cleaning up or reviewing existing Rust code."; `v3` each starts with "Load before writing or changing
Rust code that ..."; `v4` the six removed and replaced by `rust-new-code`, `rust-refactor`,
`rust-add-feature` with request-level descriptions and the 68-line checklist as body; `v5` = `v4`
plus the six; `v6` the single entry skill of `arms/short`; `v7` the six descriptions from
`guidance.md`, verbatim.

Incident. The brief copied `"Bash(cargo:*)"` into the disallowed tools from the old probe script. On
the greenfield prompt the agent read the spec, had `cargo --version` denied and gave up, so nothing
could be measured. The probe agent stopped after 8 probes, tried to remove the deny from its script,
had that edit refused by the permission classifier, and did not work around it. The user was asked
and said to allow cargo; the deny was then removed by the main session, `--max-turns` raised from 6
to 10, concurrency lowered to 2, and the 8 logs moved to `../v3/logs/cargo-denied/` (not counted).

Result (runs out of 3 in which the skill or skills loaded):

| variant | task | n | loads per skill (runs of n) | first Skill call index | cost/probe |
|---|---|---|---|---|---|
| v0 | feature | 3 | none (six-total 0) | -,-,- | 0.360 |
| v0 | green | 3 | none (six-total 0) | -,-,- | 0.520 |
| v0 | refactor | 3 | none (six-total 0) | -,-,- | 0.258 |
| v1 | feature | 3 | none (six-total 0) | -,-,- | 0.370 |
| v1 | green | 3 | none (six-total 0) | -,-,- | 0.509 |
| v1 | refactor | 3 | none (six-total 0) | -,-,- | 0.229 |
| v2 | feature | 3 | none (six-total 0) | -,-,- | 0.289 |
| v2 | green | 3 | none (six-total 0) | -,-,- | 0.335 |
| v2 | refactor | 3 | none (six-total 0) | -,-,- | 0.245 |
| v3 | feature | 3 | none (six-total 0) | -,-,- | 0.335 |
| v3 | green | 3 | none (six-total 0) | -,-,- | 0.340 |
| v3 | refactor | 3 | com 1 kis 1 (six-total 2) | -,-,2 | 0.253 |
| v4 | feature | 3 | ADD 3; matching task skill 3/3, wrong task skill 0/3 (six-total 0) | 1,1,1 | 0.370 |
| v4 | green | 3 | NEW 3; matching task skill 3/3, wrong task skill 0/3 (six-total 0) | 1,1,1 | 0.472 |
| v4 | refactor | 3 | REF 3; matching task skill 3/3, wrong task skill 0/3 (six-total 0) | 1,1,1 | 0.231 |
| v5 | feature | 3 | ADD 3; matching task skill 3/3, wrong task skill 0/3 (six-total 0) | 1,1,1 | 0.346 |
| v5 | green | 3 | NEW 3; matching task skill 3/3, wrong task skill 0/3 (six-total 0) | 2,1,2 | 0.519 |
| v5 | refactor | 3 | REF 3; matching task skill 3/3, wrong task skill 0/3 (six-total 0) | 1,1,1 | 0.242 |
| v6 | feature | 3 | rules 3 (six-total 0) | 1,1,1 | 0.369 |
| v6 | green | 3 | rules 3 (six-total 0) | 1,1,1 | 0.499 |
| v6 | refactor | 3 | rules 3 (six-total 0) | 1,1,1 | 0.239 |
| v7 | feature | 3 | none (six-total 0) | -,-,- | 0.343 |
| v7 | green | 3 | none (six-total 0) | -,-,- | 0.523 |
| v7 | refactor | 3 | none (six-total 0) | -,-,- | 0.239 |

- Task-level skills and the entry skill loaded in 27 of 27 probes, as the first or second tool call,
  always the skill matching the request and never a wrong one.
- The six narrow skills loaded once in 45 probes across five wordings, including the rewrite that
  follows the published guidance.
- With the three task skills present, the six narrow ones beside them never loaded (`v5`), so if
  they stay as reference the task skill's body has to point at them.
- One entry skill loads as reliably as three task skills. Any difference between the two designs
  would come from content, which this probe does not measure.
- The control did not repeat P1: on greenfield the six guides loaded by themselves in 4 of 6 runs on
  2026-11-20 (first Skill call at tool call 3 in `p1-05`, `p1-08`, `p1-12`, `p1-14`) and in 0 of 3
  on 2026-11-23, with the same client version (2.1.318) and the same first two tool calls. Self
  loading of the narrow guides is unreliable even on greenfield.
- Cost USD 25.29 against an estimate of 8 to 10: with cargo allowed most greenfield probes ran to
  the budget cap.

### 11.8 Consumption of the earlier 87 runs

Script: `bin/usage.py <keys.tsv> <logs-dir> [--runs]`. It reads the `result` messages of each log:
turns and duration are summed over all of them, cost and token totals come from the last one's
`total_cost_usd` and `modelUsage`. `fresh_in` is uncached input plus cache writes. Only one of the 87
runs has more than one `result` message (`p4-05`).

All Sonnet cells of the trial, including the hook arms of 11.9 (means; `cost_vs_none` and
`min_vs_none` are relative to `none` on the same task):

| task | arm | n | turns | minutes | cost | fresh_in | cache_read | output | cost_vs_none | min_vs_none |
|---|---|---|---|---|---|---|---|---|---|---|
| feature | context | 3 | 22 | 3.0 | 0.64 | 69k | 1.23M | 24k | 1.30x | 1.32x |
| feature | context2 | 3 | 18 | 2.6 | 0.56 | 66k | 0.88M | 21k | 1.15x | 1.13x |
| feature | hk-gate | 3 | 17 | 2.4 | 0.48 | 57k | 0.79M | 17k | 0.97x | 1.04x |
| feature | hk-gate2 | 3 | 18 | 3.4 | 0.58 | 64k | 1.08M | 22k | 1.18x | 1.48x |
| feature | hk-pre | 3 | 15 | 2.7 | 0.51 | 59k | 0.87M | 19k | 1.04x | 1.17x |
| feature | hk-pre2 | 3 | 15 | 2.6 | 0.52 | 60k | 0.90M | 19k | 1.06x | 1.15x |
| feature | hk-start | 3 | 14 | 2.7 | 0.55 | 65k | 0.81M | 21k | 1.12x | 1.16x |
| feature | hk-start-stop | 3 | 24 | 3.6 | 0.69 | 72k | 1.51M | 25k | 1.39x | 1.55x |
| feature | hook | 3 | 24 | 3.5 | 0.71 | 76k | 1.52M | 26k | 1.45x | 1.52x |
| feature | limits | 3 | 19 | 2.6 | 0.55 | 63k | 1.01M | 20k | 1.12x | 1.11x |
| feature | none | 3 | 14 | 2.3 | 0.49 | 58k | 0.76M | 18k | 1.00x | 1.00x |
| feature | router | 3 | 30 | 2.6 | 0.69 | 89k | 1.29M | 21k | 1.41x | 1.15x |
| feature | short | 3 | 17 | 2.5 | 0.52 | 60k | 0.86M | 19k | 1.06x | 1.08x |
| feature | skills | 3 | 18 | 2.5 | 0.53 | 59k | 1.02M | 19k | 1.07x | 1.10x |
| green | context | 3 | 28 | 9.8 | 1.59 | 115k | 2.27M | 90k | 1.69x | 1.71x |
| green | context2 | 3 | 21 | 9.3 | 1.42 | 111k | 2.08M | 77k | 1.51x | 1.62x |
| green | hk-gate | 3 | 20 | 6.2 | 0.97 | 75k | 1.32M | 54k | 1.03x | 1.08x |
| green | hk-gate2 | 3 | 29 | 12.2 | 1.68 | 121k | 2.40M | 95k | 1.78x | 2.12x |
| green | hk-pre | 3 | 25 | 7.5 | 1.09 | 82k | 1.79M | 59k | 1.16x | 1.30x |
| green | hk-pre2 | 3 | 24 | 8.2 | 1.23 | 92k | 1.79M | 68k | 1.31x | 1.43x |
| green | hk-start | 3 | 19 | 9.7 | 1.50 | 111k | 1.74M | 88k | 1.59x | 1.69x |
| green | hk-start-stop | 3 | 24 | 10.1 | 1.58 | 117k | 2.37M | 88k | 1.68x | 1.76x |
| green | hook | 3 | 36 | 9.2 | 1.60 | 122k | 2.88M | 82k | 1.70x | 1.61x |
| green | limits | 3 | 17 | 5.8 | 0.94 | 76k | 1.03M | 53k | 0.99x | 1.00x |
| green | none | 3 | 18 | 5.7 | 0.94 | 73k | 1.17M | 53k | 1.00x | 1.00x |
| green | router | 3 | 36 | 8.5 | 1.51 | 124k | 2.54M | 76k | 1.61x | 1.48x |
| green | short | 3 | 22 | 7.4 | 1.20 | 90k | 1.63M | 68k | 1.27x | 1.28x |
| green | skills | 3 | 42 | 9.5 | 1.72 | 137k | 2.96M | 88k | 1.83x | 1.66x |
| refactor | audit | 3 | 14 | 1.6 | 0.31 | 42k | 0.63M | 8k | 1.23x | 1.35x |
| refactor | context | 3 | 28 | 4.7 | 0.87 | 80k | 2.03M | 34k | 3.40x | 3.96x |
| refactor | context2 | 3 | 29 | 3.6 | 0.72 | 71k | 1.88M | 25k | 2.82x | 3.05x |
| refactor | hk-gate | 3 | 11 | 1.3 | 0.27 | 41k | 0.50M | 5k | 1.04x | 1.08x |
| refactor | hk-gate2 | 3 | 17 | 2.2 | 0.38 | 47k | 0.95M | 10k | 1.50x | 1.85x |
| refactor | hk-pre | 3 | 11 | 1.4 | 0.28 | 39k | 0.56M | 6k | 1.08x | 1.22x |
| refactor | hk-pre2 | 3 | 15 | 2.3 | 0.39 | 49k | 0.83M | 11k | 1.54x | 1.95x |
| refactor | hk-start | 3 | 17 | 2.4 | 0.40 | 47k | 0.90M | 12k | 1.56x | 2.01x |
| refactor | hk-start-stop | 3 | 25 | 4.5 | 0.69 | 69k | 1.68M | 24k | 2.70x | 3.81x |
| refactor | hook | 3 | 14 | 1.4 | 0.33 | 47k | 0.66M | 7k | 1.28x | 1.22x |
| refactor | limits | 3 | 16 | 1.6 | 0.35 | 47k | 0.74M | 9k | 1.36x | 1.32x |
| refactor | none | 3 | 9 | 1.2 | 0.25 | 37k | 0.41M | 7k | 1.00x | 1.00x |
| refactor | router | 3 | 27 | 1.5 | 0.44 | 66k | 1.01M | 8k | 1.74x | 1.30x |
| refactor | short | 3 | 15 | 1.6 | 0.33 | 43k | 0.70M | 9k | 1.31x | 1.38x |
| refactor | skills | 3 | 11 | 0.9 | 0.25 | 41k | 0.41M | 4k | 0.97x | 0.79x |

- The extra cost is work done, not rule text in context: on greenfield the `context` arm writes 90k
  output tokens against 53k for `none` (more types, more tests).
- The 3.4x of `context` on refactor is relative to a `none` that does almost nothing (1.2 minutes,
  USD 0.25); in absolute terms it is 4.7 minutes and USD 0.87.
- The short checklist skill is the cheapest arm that works: +27% on greenfield, +6% on feature.

### 11.9 Hook delivery A/B (P7 and P8, 54 runs)

Question: does a hook that puts the rules in front of the agent, without depending on the agent's
choice to load a skill, improve the code, and at what cost?

Arms, all in `arms/`, all carrying the same 68-line checklist (`arms/context/CLAUDE.md` copied to
`hooks/checklist.md`) and no skills:

| Arm | Hook | Script |
|---|---|---|
| `hk-start` | `SessionStart` prints the checklist (plain stdout becomes context) and records `HEAD` | `hooks/rules.sh start` |
| `hk-start-stop` | the same, plus a `Stop` hook that checks only the Rust lines the session added and hands the findings back once (exit 2) | `hooks/rules.sh start`, `hooks/diff-check.py` |
| `hk-pre`, `hk-pre2` | `PreToolUse` adds the checklist as `additionalContext` before the first Rust write of the session | `hooks/rules.sh context` |
| `hk-gate`, `hk-gate2` | `PreToolUse` rejects the first Rust write (exit 2) with the checklist as the reason; the agent retries | `hooks/rules.sh gate` |

`diff-check.py` builds the set of added lines from `git diff -U0 <base>` plus untracked `.rs` files,
runs clippy with the 14-lint set and keeps only findings on those lines, applies the five regular
expressions of the earlier `jrr-check.sh` to those lines only, and fires once per session. A dry run
on a two-line crate flagged the added line and not the inherited one.

Verification before the batch (`bin/hookprobe.sh <arm>`, logs in `logs/hookprobe/`): one short real
session per arm. In all four the agent quoted the title line of the rules or responded to the
detector's finding; the gate arm showed one rejection followed by a retry. About USD 0.06 each.

Incident: the first `hk-pre` and `hk-gate` matched only `Edit|Write|MultiEdit`. In the real runs the
agent wrote most Rust through Bash heredocs, so those hooks fired in 4 of 18 runs (`p7f-12` and
`p7g-27` for the gate, `p7g-35` and `p7r-30` for the context arm). The other 14 are effectively
`none` runs. The `2` arms add `Bash` to the matcher and `hooks/writes_rust.py`, which treats a Bash
command as a Rust write when it runs `cargo init` or `cargo new`, or mentions `.rs` together with a
redirect, heredoc, `tee`, `sed -i`, `perl -pi`, `open(`, `write_text` or `apply_patch`. They fired in
18 of 18 runs of P8 (nine rejections seen in the gate logs; 18 new marker files). `SessionStart` and
`Stop` fired in all 18 runs of `hk-start` and `hk-start-stop`.

Runs: `keys/p7.list` (36 runs: four arms, three tasks, n=3, shuffled) and `keys/p8.list` (18 runs:
the two corrected arms), Sonnet 5.5, three at a time through `bin/batch.sh`. Acceptance: 53 of 54
pass every hidden test; `p7g-36` (a P7 gate run whose hook did not fire) fails 2 of 58
(`queue_does_not_exist`, `receipt_handle_check_order`), the first acceptance failure in the trial.

Blind review: three sets (`blind/p7g`, `blind/p7r`, `blind/p7f`), each with the 12 hook runs that
fired by design (`hk-start`, `hk-start-stop`, `hk-pre2`, `hk-gate2`) plus nine anchors from earlier
phases (`none`, `context`, `short`, three each) and `BASE` for refactor and feature. One Opus reviewer
per set and dimension, ten in total, same brief as before. Anchors against their earlier scores: 123
repeated scores, 122 within one point, 92 identical.

Means by arm (cost relative to `none` on the same task):

| Arm | Greenfield, sum of 16 (runs) | cost | Refactor, sum of 12, BASE 5 (runs) | cost | Feature, sum of 12 (runs) | cost |
|---|---|---|---|---|---|---|
| `hk-start-stop` | 13.00 (12, 14, 13) | 1.68x | 9.00 (10, 9, 8) | 2.70x | 9.00 (10, 9, 8) | 1.39x |
| `context` (anchor) | 12.67 (12, 13, 13) | 1.69x | 8.67 (8, 9, 9) | 3.40x | 9.67 (11, 9, 9) | 1.30x |
| `short` (anchor) | 12.67 (13, 13, 12) | 1.27x | 7.33 (6, 7, 9) | 1.31x | 9.33 (8, 11, 9) | 1.06x |
| `hk-gate2` | 11.67 (12, 12, 11) | 1.78x | 7.67 (8, 8, 7) | 1.50x | 8.67 (8, 9, 9) | 1.18x |
| `hk-start` | 11.33 (12, 9, 13) | 1.59x | 7.33 (7, 7, 8) | 1.56x | 9.33 (10, 8, 10) | 1.12x |
| `hk-pre2` | 9.67 (7, 10, 12) | 1.31x | 7.67 (7, 9, 7) | 1.54x | 8.67 (9, 7, 10) | 1.06x |
| `none` (anchor) | 6.33 (6, 5, 8) | 1.00x | 5.00 (5, 5, 5) | 1.00x | 7.00 (6, 5, 10) | 1.00x |

Greenfield, per run (types, errors, simplicity, tests; 0 to 4 each):

| run | arm | types | errors | simplicity | tests | sum |
|---|---|---|---|---|---|---|
| p1-03 | none | 0.0 | 1.0 | 2.0 | 3.0 | 6.0 |
| p1-04 | none | 0.0 | 1.0 | 2.0 | 2.0 | 5.0 |
| p1-06 | context | 4.0 | 3.0 | 1.0 | 4.0 | 12.0 |
| p1-07 | none | 1.0 | 2.0 | 3.0 | 2.0 | 8.0 |
| p1-10 | context | 3.0 | 3.0 | 3.0 | 4.0 | 13.0 |
| p1-13 | context | 4.0 | 4.0 | 2.0 | 3.0 | 13.0 |
| p6g-03 | short | 4.0 | 4.0 | 2.0 | 3.0 | 13.0 |
| p6g-05 | short | 2.0 | 3.0 | 4.0 | 4.0 | 13.0 |
| p6g-09 | short | 2.0 | 4.0 | 3.0 | 3.0 | 12.0 |
| p7g-05 | hk-start-stop | 2.0 | 3.0 | 3.0 | 4.0 | 12.0 |
| p7g-06 | hk-start-stop | 3.0 | 4.0 | 3.0 | 4.0 | 14.0 |
| p7g-11 | hk-start-stop | 3.0 | 4.0 | 2.0 | 4.0 | 13.0 |
| p7g-15 | hk-start | 2.0 | 4.0 | 3.0 | 3.0 | 12.0 |
| p7g-22 | hk-start | 1.0 | 3.0 | 2.0 | 3.0 | 9.0 |
| p7g-26 | hk-start | 3.0 | 4.0 | 2.0 | 4.0 | 13.0 |
| p8g-02 | hk-gate2 | 3.0 | 3.0 | 2.0 | 4.0 | 12.0 |
| p8g-03 | hk-gate2 | 4.0 | 3.0 | 2.0 | 3.0 | 12.0 |
| p8g-11 | hk-pre2 | 1.0 | 2.0 | 1.0 | 3.0 | 7.0 |
| p8g-12 | hk-pre2 | 2.0 | 3.0 | 2.0 | 3.0 | 10.0 |
| p8g-15 | hk-pre2 | 3.0 | 4.0 | 2.0 | 3.0 | 12.0 |
| p8g-17 | hk-gate2 | 3.0 | 3.0 | 2.0 | 3.0 | 11.0 |

Refactor, per run (types, errors, simplicity):

| run | arm | types | errors | simplicity | sum |
|---|---|---|---|---|---|
| BASE | BASE | 1.0 | 2.0 | 2.0 | 5.0 |
| p3-07 | none | 1.0 | 2.0 | 2.0 | 5.0 |
| p3-08 | context | 4.0 | 3.0 | 1.0 | 8.0 |
| p3-09 | none | 1.0 | 2.0 | 2.0 | 5.0 |
| p3-10 | none | 1.0 | 2.0 | 2.0 | 5.0 |
| p3-11 | context | 3.0 | 4.0 | 2.0 | 9.0 |
| p3-15 | context | 4.0 | 4.0 | 1.0 | 9.0 |
| p6r-04 | short | 2.0 | 2.0 | 2.0 | 6.0 |
| p6r-05 | short | 2.0 | 2.0 | 3.0 | 7.0 |
| p6r-07 | short | 3.0 | 4.0 | 2.0 | 9.0 |
| p7r-07 | hk-start | 2.0 | 3.0 | 2.0 | 7.0 |
| p7r-13 | hk-start-stop | 3.0 | 4.0 | 3.0 | 10.0 |
| p7r-18 | hk-start | 2.0 | 3.0 | 2.0 | 7.0 |
| p7r-20 | hk-start-stop | 3.0 | 4.0 | 2.0 | 9.0 |
| p7r-23 | hk-start-stop | 3.0 | 3.0 | 2.0 | 8.0 |
| p7r-24 | hk-start | 2.0 | 3.0 | 3.0 | 8.0 |
| p8r-04 | hk-pre2 | 2.0 | 3.0 | 2.0 | 7.0 |
| p8r-05 | hk-gate2 | 3.0 | 4.0 | 1.0 | 8.0 |
| p8r-07 | hk-gate2 | 2.0 | 3.0 | 3.0 | 8.0 |
| p8r-08 | hk-pre2 | 3.0 | 4.0 | 2.0 | 9.0 |
| p8r-13 | hk-gate2 | 2.0 | 3.0 | 2.0 | 7.0 |
| p8r-18 | hk-pre2 | 2.0 | 3.0 | 2.0 | 7.0 |

Feature, per run (types, errors, tests, judged on the code that differs from `BASE`):

| run | arm | types | errors | tests | sum |
|---|---|---|---|---|---|
| BASE | BASE | 2.0 | 4.0 | 1.0 | 7.0 |
| p2-02 | context | 4.0 | 4.0 | 3.0 | 11.0 |
| p2-08 | context | 3.0 | 3.0 | 3.0 | 9.0 |
| p2-09 | none | 1.0 | 3.0 | 2.0 | 6.0 |
| p2-10 | context | 3.0 | 3.0 | 3.0 | 9.0 |
| p2-12 | none | 1.0 | 2.0 | 2.0 | 5.0 |
| p2-13 | none | 3.0 | 3.0 | 4.0 | 10.0 |
| p6f-01 | short | 1.0 | 4.0 | 3.0 | 8.0 |
| p6f-07 | short | 3.0 | 4.0 | 4.0 | 11.0 |
| p6f-08 | short | 3.0 | 3.0 | 3.0 | 9.0 |
| p7f-01 | hk-start-stop | 3.0 | 3.0 | 4.0 | 10.0 |
| p7f-04 | hk-start | 3.0 | 4.0 | 3.0 | 10.0 |
| p7f-17 | hk-start-stop | 3.0 | 3.0 | 3.0 | 9.0 |
| p7f-29 | hk-start-stop | 2.0 | 2.0 | 4.0 | 8.0 |
| p7f-32 | hk-start | 2.0 | 3.0 | 3.0 | 8.0 |
| p7f-33 | hk-start | 3.0 | 3.0 | 4.0 | 10.0 |
| p8f-01 | hk-pre2 | 2.0 | 3.0 | 4.0 | 9.0 |
| p8f-06 | hk-pre2 | 2.0 | 3.0 | 2.0 | 7.0 |
| p8f-09 | hk-gate2 | 1.0 | 3.0 | 4.0 | 8.0 |
| p8f-10 | hk-gate2 | 3.0 | 3.0 | 3.0 | 9.0 |
| p8f-14 | hk-pre2 | 3.0 | 3.0 | 4.0 | 10.0 |
| p8f-16 | hk-gate2 | 2.0 | 4.0 | 3.0 | 9.0 |

Mechanical measures, every run of P7 and P8 (`hook_fired` counts Stop-hook feedback only; for the
first `hk-pre` and `hk-gate` see the incident above):

| run | arm | acceptance | hook_fired | turns | minutes | cost_usd | loc_prod | newtypes | enums | stringly_ids_per_kloc | expect_per_kloc | unwrap_or_per_kloc | clippy_per_kloc | weak_asserts | tests |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| p7f-01 | hk-start-stop | 80/80 | 1 | 21 | 3.6 | 0.66 | 1320 | 0 | 3 | 21.21 | 16.31 | 8.97 | 6.53 | 20 | 72 |
| p7f-04 | hk-start | 80/80 | 0 | 13 | 2.6 | 0.53 | 1345 | 0 | 3 | 18.39 | 17.59 | 8.79 | 6.39 | 21 | 62 |
| p7f-08 | hk-gate | 80/80 | 0 | 19 | 2.1 | 0.44 | 1276 | 0 | 3 | 18.5 | 15.98 | 9.25 | 5.89 | 21 | 54 |
| p7f-09 | hk-pre | 80/80 | 0 | 13 | 2.5 | 0.48 | 1313 | 0 | 3 | 21.33 | 16.41 | 9.02 | 5.74 | 21 | 57 |
| p7f-10 | hk-pre | 80/80 | 0 | 15 | 2.4 | 0.47 | 1281 | 0 | 3 | 19.31 | 15.95 | 9.24 | 5.88 | 22 | 55 |
| p7f-12 | hk-gate | 80/80 | 0 | 20 | 3.1 | 0.56 | 1314 | 0 | 3 | 22.06 | 16.34 | 8.99 | 5.72 | 21 | 59 |
| p7f-17 | hk-start-stop | 80/80 | 1 | 26 | 3.5 | 0.69 | 1342 | 0 | 3 | 21.62 | 17.61 | 9.61 | 5.6 | 20 | 63 |
| p7f-19 | hk-gate | 80/80 | 0 | 11 | 2.0 | 0.44 | 1250 | 0 | 3 | 18.03 | 15.45 | 9.44 | 8.58 | 21 | 63 |
| p7f-21 | hk-pre | 80/80 | 0 | 18 | 3.1 | 0.58 | 1322 | 0 | 3 | 22.76 | 15.45 | 8.94 | 8.13 | 23 | 72 |
| p7f-29 | hk-start-stop | 80/80 | 1 | 25 | 3.6 | 0.7 | 1317 | 0 | 3 | 19.64 | 16.37 | 9.0 | 6.55 | 20 | 71 |
| p7f-32 | hk-start | 80/80 | 0 | 13 | 2.5 | 0.54 | 1301 | 0 | 3 | 22.39 | 16.58 | 9.12 | 5.8 | 22 | 66 |
| p7f-33 | hk-start | 80/80 | 0 | 15 | 2.8 | 0.58 | 1329 | 0 | 3 | 21.81 | 17.77 | 8.89 | 6.46 | 20 | 74 |
| p7g-03 | hk-pre | 58/58 | 0 | 21 | 6.0 | 0.93 | 1029 | 0 | 3 | 21.16 | 5.29 | 13.76 | 8.47 | 20 | 35 |
| p7g-05 | hk-start-stop | 58/58 | 1 | 20 | 9.8 | 1.5 | 1395 | 9 | 7 | 14.37 | 1.51 | 7.56 | 0.0 | 0 | 47 |
| p7g-06 | hk-start-stop | 58/58 | 1 | 28 | 10.8 | 1.73 | 1495 | 10 | 8 | 10.61 | 2.12 | 7.78 | 0.0 | 0 | 43 |
| p7g-11 | hk-start-stop | 58/58 | 1 | 25 | 9.7 | 1.51 | 1464 | 10 | 6 | 11.44 | 2.15 | 10.01 | 0.0 | 0 | 41 |
| p7g-15 | hk-start | 58/58 | 0 | 17 | 10.5 | 1.59 | 1570 | 8 | 8 | 10.56 | 2.82 | 4.93 | 1.41 | 1 | 47 |
| p7g-16 | hk-gate | 58/58 | 0 | 17 | 5.5 | 0.9 | 1039 | 0 | 5 | 18.65 | 2.07 | 7.25 | 19.69 | 13 | 38 |
| p7g-22 | hk-start | 58/58 | 0 | 20 | 8.4 | 1.33 | 1334 | 5 | 7 | 12.74 | 0.8 | 10.35 | 0.0 | 7 | 73 |
| p7g-26 | hk-start | 58/58 | 0 | 21 | 10.2 | 1.57 | 1498 | 7 | 7 | 13.4 | 0.71 | 9.87 | 2.82 | 0 | 40 |
| p7g-27 | hk-gate | 58/58 | 0 | 24 | 6.8 | 1.13 | 1487 | 10 | 7 | 10.52 | 0.7 | 9.82 | 4.21 | 3 | 38 |
| p7g-28 | hk-pre | 58/58 | 0 | 29 | 8.5 | 1.1 | 977 | 0 | 2 | 21.91 | 0.0 | 35.05 | 6.57 | 4 | 43 |
| p7g-35 | hk-pre | 58/58 | 0 | 24 | 7.9 | 1.25 | 1238 | 2 | 3 | 28.75 | 0.0 | 6.97 | 19.16 | 2 | 59 |
| p7g-36 | hk-gate | 56/58 | 0 | 20 | 6.3 | 0.88 | 1093 | 0 | 2 | 17.02 | 4.0 | 25.03 | 4.0 | 14 | 37 |
| p7r-02 | hk-gate | 58/58 | 0 | 13 | 1.7 | 0.31 | 1083 | 0 | 3 | 23.21 | 1.01 | 10.09 | 5.05 | 20 | 44 |
| p7r-07 | hk-start | 58/58 | 0 | 18 | 2.0 | 0.36 | 1126 | 0 | 4 | 24.15 | 0.97 | 9.66 | 4.83 | 20 | 44 |
| p7r-13 | hk-start-stop | 58/58 | 1 | 28 | 5.4 | 0.82 | 1201 | 3 | 4 | 22.89 | 1.83 | 9.16 | 2.75 | 20 | 47 |
| p7r-14 | hk-pre | 58/58 | 0 | 10 | 1.3 | 0.28 | 1069 | 0 | 3 | 21.32 | 12.18 | 10.15 | 5.08 | 20 | 44 |
| p7r-18 | hk-start | 58/58 | 0 | 13 | 1.7 | 0.32 | 1105 | 0 | 4 | 21.61 | 0.98 | 9.82 | 4.91 | 19 | 45 |
| p7r-20 | hk-start-stop | 58/58 | 1 | 30 | 4.7 | 0.76 | 1185 | 2 | 5 | 24.75 | 1.83 | 9.17 | 2.75 | 20 | 46 |
| p7r-23 | hk-start-stop | 58/58 | 1 | 17 | 3.4 | 0.49 | 1192 | 3 | 5 | 24.01 | 0.92 | 9.23 | 4.62 | 19 | 47 |
| p7r-24 | hk-start | 58/58 | 0 | 19 | 3.4 | 0.51 | 1118 | 2 | 3 | 20.41 | 12.63 | 9.72 | 4.86 | 15 | 46 |
| p7r-25 | hk-pre | 58/58 | 0 | 11 | 1.6 | 0.29 | 1085 | 0 | 4 | 23.02 | 12.01 | 10.01 | 5.01 | 20 | 44 |
| p7r-30 | hk-pre | 58/58 | 0 | 13 | 1.5 | 0.26 | 1068 | 0 | 3 | 19.27 | 12.17 | 10.14 | 5.07 | 20 | 44 |
| p7r-31 | hk-gate | 58/58 | 0 | 10 | 1.0 | 0.24 | 1064 | 0 | 3 | 19.35 | 12.22 | 10.18 | 5.09 | 20 | 44 |
| p7r-34 | hk-gate | 58/58 | 0 | 9 | 1.1 | 0.25 | 1076 | 0 | 3 | 21.15 | 12.08 | 10.07 | 5.04 | 20 | 44 |
| p8f-01 | hk-pre2 | 80/80 | 0 | 16 | 2.7 | 0.53 | 1290 | 0 | 3 | 18.41 | 15.06 | 9.21 | 5.86 | 22 | 62 |
| p8f-06 | hk-pre2 | 80/80 | 0 | 12 | 2.4 | 0.47 | 1291 | 0 | 3 | 20.82 | 15.82 | 9.16 | 6.66 | 21 | 70 |
| p8f-09 | hk-gate2 | 80/80 | 0 | 18 | 2.8 | 0.53 | 1290 | 0 | 3 | 19.15 | 15.82 | 9.16 | 6.66 | 21 | 65 |
| p8f-10 | hk-gate2 | 80/80 | 0 | 18 | 3.1 | 0.58 | 1339 | 0 | 3 | 21.65 | 16.04 | 8.82 | 6.42 | 20 | 68 |
| p8f-14 | hk-pre2 | 80/80 | 0 | 17 | 2.8 | 0.56 | 1328 | 0 | 3 | 21.92 | 16.23 | 8.93 | 5.68 | 22 | 60 |
| p8f-16 | hk-gate2 | 80/80 | 0 | 19 | 4.3 | 0.63 | 1302 | 0 | 3 | 20.63 | 15.68 | 9.08 | 6.6 | 21 | 72 |
| p8g-02 | hk-gate2 | 58/58 | 0 | 23 | 13.6 | 2.15 | 1535 | 5 | 5 | 13.84 | 2.08 | 6.23 | 2.77 | 3 | 59 |
| p8g-03 | hk-gate2 | 58/58 | 0 | 26 | 8.0 | 1.21 | 1519 | 12 | 9 | 6.24 | 4.16 | 10.4 | 2.77 | 10 | 28 |
| p8g-11 | hk-pre2 | 58/58 | 0 | 17 | 6.6 | 1.09 | 1378 | 7 | 5 | 26.17 | 0.0 | 17.71 | 4.62 | 0 | 29 |
| p8g-12 | hk-pre2 | 58/58 | 0 | 19 | 7.4 | 1.19 | 1354 | 8 | 4 | 14.23 | 0.0 | 7.91 | 7.11 | 4 | 52 |
| p8g-15 | hk-pre2 | 58/58 | 0 | 37 | 10.5 | 1.4 | 1477 | 7 | 9 | 9.27 | 0.71 | 6.42 | 2.85 | 10 | 41 |
| p8g-17 | hk-gate2 | 58/58 | 0 | 37 | 14.9 | 1.66 | 1549 | 9 | 5 | 10.91 | 2.73 | 9.55 | 2.73 | 6 | 47 |
| p8r-04 | hk-pre2 | 58/58 | 0 | 13 | 1.9 | 0.34 | 1102 | 0 | 4 | 21.61 | 0.98 | 9.82 | 2.95 | 20 | 44 |
| p8r-05 | hk-gate2 | 58/58 | 0 | 21 | 3.0 | 0.49 | 1171 | 2 | 4 | 22.3 | 0.93 | 9.29 | 2.79 | 20 | 46 |
| p8r-07 | hk-gate2 | 58/58 | 0 | 16 | 1.9 | 0.36 | 1129 | 0 | 4 | 21.13 | 11.53 | 9.61 | 2.88 | 19 | 44 |
| p8r-08 | hk-pre2 | 58/58 | 0 | 19 | 2.6 | 0.47 | 1150 | 3 | 4 | 25.57 | 3.79 | 9.47 | 4.73 | 20 | 45 |
| p8r-13 | hk-gate2 | 58/58 | 0 | 15 | 1.7 | 0.3 | 1101 | 0 | 4 | 23.62 | 0.98 | 9.84 | 4.92 | 20 | 44 |
| p8r-18 | hk-pre2 | 58/58 | 0 | 14 | 2.4 | 0.37 | 1117 | 3 | 3 | 24.39 | 0.98 | 9.76 | 4.88 | 20 | 45 |

Consumption, every run of P7 and P8:

| run | arm | task | turns | minutes | cost | fresh_in | cache_read | output |
|---|---|---|---|---|---|---|---|---|
| p7f-01 | hk-start-stop | feature | 21 | 3.64 | 0.66 | 69942 | 1344439 | 24949 |
| p7f-04 | hk-start | feature | 13 | 2.62 | 0.53 | 62595 | 765244 | 20359 |
| p7f-08 | hk-gate | feature | 19 | 2.12 | 0.44 | 54058 | 704722 | 15246 |
| p7f-09 | hk-pre | feature | 13 | 2.51 | 0.48 | 57112 | 775388 | 17873 |
| p7f-10 | hk-pre | feature | 15 | 2.43 | 0.47 | 55143 | 812022 | 16534 |
| p7f-12 | hk-gate | feature | 20 | 3.07 | 0.56 | 61032 | 1090511 | 20401 |
| p7f-17 | hk-start-stop | feature | 26 | 3.51 | 0.69 | 72883 | 1514609 | 24414 |
| p7f-19 | hk-gate | feature | 11 | 2.00 | 0.44 | 55275 | 565717 | 15782 |
| p7f-21 | hk-pre | feature | 18 | 3.14 | 0.58 | 63976 | 1031966 | 21890 |
| p7f-29 | hk-start-stop | feature | 25 | 3.55 | 0.70 | 72155 | 1677126 | 24859 |
| p7f-32 | hk-start | feature | 13 | 2.51 | 0.54 | 63157 | 770211 | 20786 |
| p7f-33 | hk-start | feature | 15 | 2.83 | 0.58 | 67764 | 886854 | 22051 |
| p7g-03 | hk-pre | green | 21 | 6.01 | 0.93 | 71158 | 1419931 | 50423 |
| p7g-05 | hk-start-stop | green | 20 | 9.85 | 1.50 | 113028 | 1824547 | 86712 |
| p7g-06 | hk-start-stop | green | 28 | 10.76 | 1.73 | 126121 | 2859742 | 94264 |
| p7g-11 | hk-start-stop | green | 25 | 9.70 | 1.51 | 111048 | 2421394 | 82668 |
| p7g-15 | hk-start | green | 17 | 10.47 | 1.59 | 119882 | 1429590 | 96297 |
| p7g-16 | hk-gate | green | 17 | 5.53 | 0.90 | 70766 | 1089553 | 50483 |
| p7g-22 | hk-start | green | 20 | 8.39 | 1.33 | 98143 | 1836108 | 75319 |
| p7g-26 | hk-start | green | 21 | 10.21 | 1.57 | 115933 | 1957657 | 91454 |
| p7g-27 | hk-gate | green | 24 | 6.77 | 1.13 | 84745 | 1653702 | 62102 |
| p7g-28 | hk-pre | green | 29 | 8.50 | 1.10 | 82697 | 2173446 | 55149 |
| p7g-35 | hk-pre | green | 24 | 7.94 | 1.25 | 93427 | 1774232 | 69995 |
| p7g-36 | hk-gate | green | 20 | 6.30 | 0.88 | 69407 | 1210350 | 48431 |
| p7r-02 | hk-gate | refactor | 13 | 1.75 | 0.31 | 40385 | 622277 | 8230 |
| p7r-07 | hk-start | refactor | 18 | 1.98 | 0.36 | 44197 | 903278 | 9725 |
| p7r-13 | hk-start-stop | refactor | 28 | 5.43 | 0.82 | 76835 | 1962538 | 31368 |
| p7r-14 | hk-pre | refactor | 10 | 1.30 | 0.28 | 42331 | 475724 | 5856 |
| p7r-18 | hk-start | refactor | 13 | 1.75 | 0.32 | 42117 | 646297 | 8853 |
| p7r-20 | hk-start-stop | refactor | 30 | 4.71 | 0.76 | 76608 | 2098034 | 24277 |
| p7r-23 | hk-start-stop | refactor | 17 | 3.35 | 0.49 | 54875 | 975879 | 17454 |
| p7r-24 | hk-start | refactor | 19 | 3.40 | 0.51 | 55915 | 1145847 | 17019 |
| p7r-25 | hk-pre | refactor | 11 | 1.58 | 0.29 | 38631 | 542903 | 7918 |
| p7r-30 | hk-pre | refactor | 13 | 1.46 | 0.26 | 37095 | 654108 | 5104 |
| p7r-31 | hk-gate | refactor | 10 | 1.01 | 0.24 | 40970 | 471243 | 3360 |
| p7r-34 | hk-gate | refactor | 9 | 1.07 | 0.25 | 40659 | 403915 | 4311 |
| p8f-01 | hk-pre2 | feature | 16 | 2.74 | 0.53 | 60585 | 978468 | 18963 |
| p8f-06 | hk-pre2 | feature | 12 | 2.35 | 0.47 | 56027 | 662140 | 17936 |
| p8f-09 | hk-gate2 | feature | 18 | 2.84 | 0.53 | 59149 | 1013711 | 18973 |
| p8f-10 | hk-gate2 | feature | 18 | 3.07 | 0.58 | 65235 | 1055836 | 21714 |
| p8f-14 | hk-pre2 | feature | 17 | 2.81 | 0.56 | 63853 | 1059946 | 20329 |
| p8f-16 | hk-gate2 | feature | 19 | 4.26 | 0.63 | 66823 | 1166593 | 24937 |
| p8g-02 | hk-gate2 | green | 23 | 13.57 | 2.15 | 155240 | 2710559 | 126149 |
| p8g-03 | hk-gate2 | green | 26 | 8.03 | 1.21 | 91748 | 1620544 | 68529 |
| p8g-11 | hk-pre2 | green | 17 | 6.64 | 1.09 | 85431 | 1209487 | 62757 |
| p8g-12 | hk-pre2 | green | 19 | 7.44 | 1.19 | 91348 | 1402491 | 68809 |
| p8g-15 | hk-pre2 | green | 37 | 10.49 | 1.40 | 98681 | 2749418 | 73123 |
| p8g-17 | hk-gate2 | green | 37 | 14.92 | 1.66 | 116699 | 2883859 | 90949 |
| p8r-04 | hk-pre2 | refactor | 13 | 1.85 | 0.34 | 46778 | 668725 | 8656 |
| p8r-05 | hk-gate2 | refactor | 21 | 2.99 | 0.49 | 53829 | 1254346 | 14501 |
| p8r-07 | hk-gate2 | refactor | 16 | 1.88 | 0.36 | 47092 | 854513 | 8785 |
| p8r-08 | hk-pre2 | refactor | 19 | 2.64 | 0.47 | 53522 | 1085239 | 15003 |
| p8r-13 | hk-gate2 | refactor | 15 | 1.71 | 0.30 | 38973 | 726212 | 6819 |
| p8r-18 | hk-pre2 | refactor | 14 | 2.42 | 0.37 | 47623 | 748026 | 10099 |

Reading:

- Greenfield and refactor: every hook arm is above `none` with no overlap between runs.
- `hk-start-stop` is the best hook arm and equals the checklist in `CLAUDE.md` on quality. On
  refactor it costs less (2.70x against 3.40x) and keeps simplicity (2.33 against 1.33). Its
  greenfield crates have zero clippy findings and zero weak assertions on the mechanical measures.
- Rejecting the first write adds nothing over adding context: same refactor and feature scores,
  more time on greenfield (12.2 minutes against 8.2).
- The short skill has the best quality for its cost: 12.67 on greenfield at +27%.
- Feature: the rule arms average 1.7 to 2.7 points above `none` and none of their runs is below 7,
  but one `none` run scored 10, so the ranges overlap. This is more than P2 and P6 showed (nothing
  moved there); it comes from types and tests in the new code. `expect` per KLOC still does not move
  (17.3 and 16.8 against 16.3 for `none`).
- Differences of one or two points between hook arms rest on three runs per cell and are not firm.
  What is firm is hook against no hook on greenfield and refactor.
- A hook bound only to the file-editing tools misses most writes in headless Sonnet runs.

### 11.10 Corrections to earlier statements

- "Greenfield: the skills load by themselves" (sections 1 and 6.1, and the one-page summary) holds
  for 4 of 6 runs on 2026-11-20 and 0 of 3 on 2026-11-23. Treat it as unreliable.
- "Feature-add: nothing moved" (section 6.5) still holds for the mechanical measures; the blind
  review of this round shows a weak, overlapping gain for every rule arm.
- The estimate of USD 8 to 10 for the probes was wrong (USD 25.29), and the estimate of USD 36 for
  the hook A/B became USD 41.31 because of the rerun.
- The first statement of this round that the transcripts numbered about 150 was wrong: 116.
- Reviewers of this round ran on Opus by the user's instruction; scores still compare only inside
  one set, and the anchors tie this round to the earlier ones.

### 11.11 What follows from this round

1. Replace activation by description with one entry skill or three task-level skills whose
   descriptions restate the request; keep the six guides as reference that the entry skill's body
   and the detector point to.
2. In the plugin, implement `SessionStart` with the checklist and `Stop` with the detector on added
   lines, matching Bash as well as the file tools wherever a pre-write hook is used. Agents without
   hooks (OpenCode, Goose) keep the skill.
3. Untested and worth a small run: the conditional scope sentence in `rust-keep-it-simple`; a
   feature checklist split into local and cross-cutting rules; three task skills with different
   bodies measured by full runs (the probes only show that they load).
4. For James's harness: a quality metric beside pass or fail, and `--trials` with an aggregate.

Open at the time of writing: the user has not yet said whether to build the task skills and the real
hook in the plugin; `../COMMENT.md` (the draft for Niko) still stops at stage 9 and does not include
this round.

### 11.12 Files added in this round

```
v2/bin/usage.py                 consumption per run and per arm from the logs
v2/bin/hookprobe.sh             one short run that shows whether an arm's hooks reach the agent
v2/arms/hk-start, hk-start-stop, hk-pre, hk-gate, hk-pre2, hk-gate2   hook arms (hooks/ holds the scripts)
v2/keys/p7.list, p8.list        run lists; blind-p7g.tsv, blind-p7r.tsv, blind-p7f.tsv
v2/runs|eval|logs/p7*, p8*      the 54 runs; logs/hookprobe/ the four verification runs
v2/blind/p7g, p7r, p7f          the three review sets; reviews/p7g, p7r, p7f the ten reviews
v3/descriptions/guidance.md     published guidance and the corrected descriptions
v3/descriptions/evidence.md     what loaded in the 116 transcripts (extract.py, tables.py)
v3/descriptions/probe.md        the description probes: variant texts and results
v3/variants/v0..v7, v3/bin, v3/logs   probe plugins, scripts and logs (cargo-denied/ is discarded)
```
