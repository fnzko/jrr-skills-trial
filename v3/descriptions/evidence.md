# Which skill descriptions got their skill loaded

Source: every headless transcript in `logs/*.jsonl` (29: runs 1 to 15, `pilot`, the narrow
`probe`, 12 trigger probes) and `v2/logs/*.jsonl` (87). That is 116 runs, not about 150. Scripts:
`v3/descriptions/extract.py` (per-run facts from the stream) and `v3/descriptions/tables.py` (this
table). Rerun with:

~~~sh
python3 -I v3/descriptions/extract.py . > runs.json
python3 -I v3/descriptions/tables.py runs.json .
~~~

## Method and caveats

- **Available** = the skill is in the `skills` list of a `system`/`init` message of the run. The
  arm comes from `mapping.tsv`, `v2/keys/runs.tsv` (last row wins for the six P1 reruns) and the
  probe file name, and was cross-checked against the `plugins[].path` of the init message.
- **Description text** = the `SKILL.md` frontmatter in the plugin directory the init message names
  (`runs/N/.claude/skills/just-right-rust`, `probe-plugins/*`, `v2/arms/*`). The skill listing
  system-reminder is **not** in these stream-json transcripts (zero matches for "available for use
  with the Skill tool"), so the exact text the model saw is not recorded; the files on disk are the
  best evidence. The arm folders were committed once and not edited afterwards.
- **Position** = 1-based index of the Skill call among the main agent's tool calls in the whole
  file (all sessions; subagent calls excluded; no subagent ever called Skill). No run read a
  `SKILL.md` with `Read`, so Skill calls are the only load path.
- Load columns split who caused the load:
  - *description alone*: no router skill listing the guide, no "Use the Rust skills" line in the
    prompt, and (hook arm) before the Stop hook fed back findings. Denominator = runs meeting that.
  - *router body asked*: arms `router`, `b2`, `b3`, whose entry skill body tells the agent to load
    the six guides.
  - *after Stop hook*: hook arm, Skill call after the hook's `just-right-rust check:` feedback (the
    feedback ends "Load the matching rust-* skill for how to fix").
  - *prompt said use skills*: v1 runs 4 to 9 (`prompt-skills.txt`).
- Task types: greenfield = P1, P4, P6g; feature-add = P2, P6f; refactor-small = P3, P6r;
  refactor-large = v1 runs 1 to 15 and `pilot`; probe = `probe-{a,b1,b2}-*` (plain refactor prompt
  on the large crate, `--max-turns 8`; the `a` and `b1` probes stopped after 4 to 6 tool calls
  because `cargo` was denied, but no load anywhere happens later than call 3 unaided, so this does
  not hide loads); probe-review = the single narrow `probe` run ("Review the error handling in
  .../send_message.rs").
- Description variants: `(v2)` = the `probe-plugins/b1` text, which appends "Also use when
  refactoring or cleaning up existing Rust code, to ..." to each guide. `rust-rules` is the same
  text in `short` and `limits`.
- "[Critical] ... Always activate" style descriptions do **not** occur in any of the 116
  transcripts (zero matches for "Always activate" and "[Critical]"). The isolated config exposed 35
  built-in Claude Code skills (listed below); no Symposium-style "always" skills. So "always"
  wording cannot be tested from this data.

## 1. Availability and loads per (description, task, model)

| Description | Task | Model | Available | Loaded, description alone | Loaded, router body asked | Loaded, after Stop hook | Loaded, prompt said use skills | Position of Skill call |
|---|---|---|---|---|---|---|---|---|
| `async-rust` | greenfield | haiku | 2 | 0 of 2 | - | - | - | - |
| `async-rust` | greenfield | opus | 2 | 0 of 2 | - | - | - | - |
| `async-rust` | greenfield | sonnet | 9 | 0 of 6 | 0 of 3 | 0 of 3 | - | - |
| `async-rust` | feature-add | sonnet | 9 | 0 of 6 | 0 of 3 | 0 of 3 | - | - |
| `async-rust` | refactor-small | sonnet | 12 | 0 of 9 | 0 of 3 | 0 of 3 | - | - |
| `async-rust` | refactor-large | sonnet | 16 | 0 of 6 | 0 of 4 | - | 0 of 6 | - |
| `async-rust` | probe | sonnet | 12 | 0 of 8 | 0 of 4 | - | - | - |
| `async-rust` | probe-review | sonnet | 1 | 0 of 1 | - | - | - | - |
| `rust-audit` | refactor-small | sonnet | 3 | 3 of 3 | - | - | - | 1, 1, 1 |
| `rust-audit` | refactor-large | sonnet | 2 | 2 of 2 | - | - | - | 1, 1 |
| `rust-code` | greenfield | sonnet | 3 | 3 of 3 | - | - | - | 1, 1, 2 |
| `rust-code` | feature-add | sonnet | 3 | 3 of 3 | - | - | - | 1, 1, 1 |
| `rust-code` | refactor-small | sonnet | 3 | 3 of 3 | - | - | - | 1, 1, 1 |
| `rust-comments` | greenfield | haiku | 2 | 1 of 2 | - | - | - | 3 |
| `rust-comments` | greenfield | opus | 2 | 2 of 2 | - | - | - | 5, 5 |
| `rust-comments` | greenfield | sonnet | 9 | 4 of 6 | 3 of 3 | 0 of 3 | - | 5, 5, 7, 7, 7, 7, 7 |
| `rust-comments` | feature-add | sonnet | 9 | 0 of 6 | 3 of 3 | 0 of 3 | - | 7, 7, 7 |
| `rust-comments` | refactor-small | sonnet | 12 | 1 of 9 | 3 of 3 | 0 of 3 | - | 3, 7, 7, 7 |
| `rust-comments` | refactor-large | sonnet | 11 | 0 of 4 | 4 of 4 | - | 2 of 3 | 3, 4, 6, 6, 7, 7 |
| `rust-comments` | probe | sonnet | 8 | 0 of 4 | 4 of 4 | - | - | 6, 6, 6, 6 |
| `rust-comments` | probe-review | sonnet | 1 | 0 of 1 | - | - | - | - |
| `rust-comments (v2)` | probe | sonnet | 4 | 0 of 4 | - | - | - | - |
| `rust-domain-types` | greenfield | haiku | 2 | 1 of 2 | - | - | - | 4 |
| `rust-domain-types` | greenfield | opus | 2 | 2 of 2 | - | - | - | 3, 3 |
| `rust-domain-types` | greenfield | sonnet | 9 | 4 of 6 | 3 of 3 | 2 of 3 | - | 3, 3, 3, 3, 3, 3, 3, 22, 25 |
| `rust-domain-types` | feature-add | sonnet | 9 | 0 of 6 | 3 of 3 | 0 of 3 | - | 3, 3, 3 |
| `rust-domain-types` | refactor-small | sonnet | 12 | 0 of 9 | 3 of 3 | 0 of 3 | - | 3, 3, 3 |
| `rust-domain-types` | refactor-large | sonnet | 12 | 0 of 5 | 4 of 4 | - | 0 of 3 | 2, 2, 3, 3 |
| `rust-domain-types` | probe | sonnet | 8 | 0 of 4 | 4 of 4 | - | - | 2, 2, 2, 2 |
| `rust-domain-types` | probe-review | sonnet | 1 | 0 of 1 | - | - | - | - |
| `rust-domain-types (v2)` | probe | sonnet | 4 | 0 of 4 | - | - | - | - |
| `rust-error-discipline` | greenfield | haiku | 2 | 1 of 2 | - | - | - | 5 |
| `rust-error-discipline` | greenfield | opus | 2 | 2 of 2 | - | - | - | 2, 2 |
| `rust-error-discipline` | greenfield | sonnet | 9 | 4 of 6 | 3 of 3 | 2 of 3 | - | 4, 4, 4, 4, 4, 4, 4, 20, 27 |
| `rust-error-discipline` | feature-add | sonnet | 9 | 0 of 6 | 3 of 3 | 1 of 3 | - | 4, 4, 4, 19 |
| `rust-error-discipline` | refactor-small | sonnet | 12 | 0 of 9 | 3 of 3 | 0 of 3 | - | 4, 4, 4 |
| `rust-error-discipline` | refactor-large | sonnet | 11 | 0 of 4 | 4 of 4 | - | 0 of 3 | 3, 3, 4, 4 |
| `rust-error-discipline` | probe | sonnet | 8 | 0 of 4 | 4 of 4 | - | - | 3, 3, 3, 3 |
| `rust-error-discipline` | probe-review | sonnet | 1 | 1 of 1 | - | - | - | 2 |
| `rust-error-discipline (v2)` | probe | sonnet | 4 | 0 of 4 | - | - | - | - |
| `rust-keep-it-simple` | greenfield | haiku | 2 | 1 of 2 | - | - | - | 7 |
| `rust-keep-it-simple` | greenfield | opus | 2 | 0 of 2 | - | - | - | - |
| `rust-keep-it-simple` | greenfield | sonnet | 9 | 4 of 6 | 3 of 3 | 0 of 3 | - | 5, 5, 6, 6, 6, 7, 7 |
| `rust-keep-it-simple` | feature-add | sonnet | 9 | 0 of 6 | 3 of 3 | 0 of 3 | - | 6, 6, 6 |
| `rust-keep-it-simple` | refactor-small | sonnet | 12 | 6 of 9 | 3 of 3 | 0 of 3 | - | 2, 2, 2, 2, 2, 2, 6, 6, 6 |
| `rust-keep-it-simple` | refactor-large | sonnet | 11 | 0 of 4 | 4 of 4 | - | 2 of 3 | 2, 3, 5, 5, 6, 6 |
| `rust-keep-it-simple` | probe | sonnet | 8 | 0 of 4 | 4 of 4 | - | - | 5, 5, 5, 5 |
| `rust-keep-it-simple` | probe-review | sonnet | 1 | 0 of 1 | - | - | - | - |
| `rust-keep-it-simple (v2)` | probe | sonnet | 4 | 0 of 4 | - | - | - | - |
| `rust-ownership` | greenfield | haiku | 2 | 0 of 2 | - | - | - | - |
| `rust-ownership` | greenfield | opus | 2 | 0 of 2 | - | - | - | - |
| `rust-ownership` | greenfield | sonnet | 9 | 2 of 6 | 3 of 3 | 0 of 3 | - | 5, 5, 5, 8, 8 |
| `rust-ownership` | feature-add | sonnet | 9 | 0 of 6 | 3 of 3 | 0 of 3 | - | 5, 5, 5 |
| `rust-ownership` | refactor-small | sonnet | 12 | 0 of 9 | 3 of 3 | 0 of 3 | - | 5, 5, 5 |
| `rust-ownership` | refactor-large | sonnet | 11 | 0 of 4 | 4 of 4 | - | 0 of 3 | 4, 4, 5, 5 |
| `rust-ownership` | probe | sonnet | 8 | 0 of 4 | 4 of 4 | - | - | 4, 4, 4, 4 |
| `rust-ownership` | probe-review | sonnet | 1 | 0 of 1 | - | - | - | - |
| `rust-ownership (v2)` | probe | sonnet | 4 | 0 of 4 | - | - | - | - |
| `rust-refactor` | refactor-large | sonnet | 4 | 4 of 4 | - | - | - | 1, 1, 1, 1 |
| `rust-refactor` | probe | sonnet | 4 | 4 of 4 | - | - | - | 1, 1, 1, 1 |
| `rust-rules` | greenfield | sonnet | 6 | 6 of 6 | - | - | - | 1, 1, 1, 1, 1, 1 |
| `rust-rules` | feature-add | sonnet | 6 | 6 of 6 | - | - | - | 1, 1, 1, 1, 1, 1 |
| `rust-rules` | refactor-small | sonnet | 6 | 6 of 6 | - | - | - | 1, 1, 1, 1, 1, 1 |
| `rust-test-quality` | greenfield | haiku | 2 | 1 of 2 | - | - | - | 6 |
| `rust-test-quality` | greenfield | opus | 2 | 2 of 2 | - | - | - | 4, 4 |
| `rust-test-quality` | greenfield | sonnet | 9 | 4 of 6 | 3 of 3 | 2 of 3 | - | 6, 6, 6, 6, 8, 8, 8, 21, 26 |
| `rust-test-quality` | feature-add | sonnet | 9 | 0 of 6 | 3 of 3 | 2 of 3 | - | 8, 8, 8, 18, 19 |
| `rust-test-quality` | refactor-small | sonnet | 12 | 0 of 9 | 3 of 3 | 0 of 3 | - | 8, 8, 8 |
| `rust-test-quality` | refactor-large | sonnet | 11 | 0 of 4 | 4 of 4 | - | 0 of 3 | 7, 7, 8, 8 |
| `rust-test-quality` | probe | sonnet | 8 | 0 of 4 | 4 of 4 | - | - | 7, 7, 7, 7 |
| `rust-test-quality` | probe-review | sonnet | 1 | 0 of 1 | - | - | - | - |
| `rust-test-quality (v2)` | probe | sonnet | 4 | 0 of 4 | - | - | - | - |
| `unsafe-rust-review` | greenfield | haiku | 2 | 0 of 2 | - | - | - | - |
| `unsafe-rust-review` | greenfield | opus | 2 | 0 of 2 | - | - | - | - |
| `unsafe-rust-review` | greenfield | sonnet | 9 | 0 of 6 | 0 of 3 | 0 of 3 | - | - |
| `unsafe-rust-review` | feature-add | sonnet | 9 | 0 of 6 | 0 of 3 | 0 of 3 | - | - |
| `unsafe-rust-review` | refactor-small | sonnet | 12 | 0 of 9 | 0 of 3 | 0 of 3 | - | - |
| `unsafe-rust-review` | refactor-large | sonnet | 16 | 0 of 6 | 0 of 4 | - | 0 of 6 | - |
| `unsafe-rust-review` | probe | sonnet | 12 | 0 of 8 | 0 of 4 | - | - | - |
| `unsafe-rust-review` | probe-review | sonnet | 1 | 0 of 1 | - | - | - | - |

Built-in skills, available in all 116 runs (every task, every model), loaded 0 times each:
`anthropic-skills:{built-in-browser, chrome-browser, computer-use, deep-research, docs, docx,
google-workspace, import-memory, morning, pdf, pptx, skill-creator, xlsx}`, `artifact-capabilities`,
`artifact-diagramming`, `batch`, `claude-api`, `code-review`, `dataviz`, `debug`, `deep-research`,
`design`, `design-sync`, `doctor`, `fewer-permission-prompts`, `loop`, `plugin-authoring`, `run`,
`run-skill-generator`, `schedule`, `simplify`, `slides`, `update-config`, `verify`,
`workflow-authoring`. Their descriptions are not in the transcripts (Claude Code 2.1.318), so they
are not classified below. The ones whose names overlap the tasks (`simplify`, `code-review`,
`verify`, `debug`) also never loaded, including in the 33 refactor runs.

Totals, description alone (all tasks): entry skills 40 of 40 (`rust-code` 9/9, `rust-audit` 5/5,
`rust-rules` 18/18, `rust-refactor` 8/8). Guides on Sonnet: `rust-keep-it-simple` 10/30,
`rust-comments` 5/30, `rust-error-discipline` 5/30, `rust-domain-types` 4/31,
`rust-test-quality` 4/30, `rust-ownership` 2/30, `async-rust` 0/36, `unsafe-rust-review` 0/36,
every `(v2)` text 0/4. Built-ins 0 of 35 x 116.

## 2. Wording against load rate

Load rate = description alone; G = greenfield (all models, 10 runs), F = feature-add (Sonnet, 6),
Rs = refactor-small (Sonnet; 6 in `skills`+`hook`, plus 3 `audit` runs where the entry skill was
loaded first), Rl = refactor-large plain prompt plus probes (8).

| Description | Chars | First sentence states | Names a whole task | Names micro-activities | Names symptoms in code | Imperative / "first" / "always" | G | F | Rs | Rl |
|---|---|---|---|---|---|---|---|---|---|---|
| `rust-code` | 217 | the request ("Write, change or refactor Rust code.") | yes, all kinds listed ("a new crate, a feature, a bug fix, a cleanup") | no | no | "Use first, before writing or editing any Rust source ... whatever the task is" | 3/3 | 3/3 | 3/3 | n/a |
| `rust-rules` | 211 | the request (same sentence) | yes, same list | no | no | same | 6/6 | 6/6 | 6/6 | n/a |
| `rust-audit` | 196 | the request ("Find what to fix in existing Rust code.") | yes ("clean up, refactor, review, tidy, simplify or improve") | no | no | "Use first for any request to ..., before reading the code in depth" | n/a | n/a | 3/3 | 2/2 |
| `rust-refactor` | 294 | the request ("Refactor or clean up existing Rust code ...") | yes ("clean up, tidy, refactor, simplify, modernize ...") | no | no | "Use first for any request to ..., before reading the code in depth" | n/a | n/a | n/a | 8/8 |
| `rust-keep-it-simple` | 459 | what it does ("Keep Rust code simple by adding no abstraction ...") | partly: "when adding a feature", "reviewing a diff" | yes (adding a trait/helper/field, writing `if`/`match`/loops) | yes (unused code, duplicated logic, shims) | no | 5/10 | 0/6 | 6/9 (6/6 without audit) | 0/8 |
| `rust-domain-types` | 318 | what it does ("Model domain data ... instead of bare strings") | no (reviewing code is tied to a symptom) | yes (defining structs, enums, signatures; parsing input) | yes (ids as `String`, `u64`, `bool`) | no | 7/10 | 0/6 | 0/9 | 0/9 (incl. pilot) |
| `rust-error-discipline` | 434 | what it does ("Handle Rust failures without hiding them.") | no | yes (handles `Result`, parses input, does I/O) | yes (`unwrap`, `expect`, `let _ =` ...) | no | 7/10 | 0/6 | 0/9 | 0/8; probe-review 1/1 |
| `rust-test-quality` | 415 | what it does ("Write Rust tests that fail when the code is wrong") | no | yes (writing `#[test]` functions, test modules) | yes (assert only `is_ok()`) | no | 7/10 | 0/6 | 0/9 | 0/8 |
| `rust-comments` | 381 | what it does ("Write Rust comments, doc comments, commit messages ...") | partly: "when committing or describing a pull request" (every prompt asks for commits) | yes (adding `//` comments) | yes (comments that narrate) | no | 7/10 | 0/6 | 1/9 | 0/8 |
| `rust-ownership` | 410 | what it does ("Avoid needless clones ...") | no | yes (signatures, getters, loops, locks) | yes (borrow checker error, `.clone()`, scan in a loop) | no | 2/10 | 0/6 | 0/9 | 0/8 |
| `(v2)` guides (b1) | 438 to 587 | what it does (unchanged) | yes, appended last: "Also use when refactoring or cleaning up existing Rust code" | yes | yes | no | n/a | n/a | n/a | 0/4 each |
| `async-rust` | 142 | the whole description | yes but scoped to a domain the code lacks ("Design, implement, debug, or review asynchronous Rust") | no | no | no | 0/10 | 0/6 | 0/9 | 0/8 |
| `unsafe-rust-review` | 290 | the whole description | scoped ("Write and review unsafe Rust `# Safety` documentation") | no | no | no | 0/10 | 0/6 | 0/9 | 0/8 |

What the rows show:

- The four 100% descriptions share five traits: the first sentence is the user's request in the
  user's verbs; they enumerate the kinds of request ("whatever the task is", "for any request
  to"); they say **first** and give an ordering anchor ("before writing or editing any Rust
  source", "before reading the code in depth"); they are 196 to 294 characters; they name no
  micro-activity and no symptom. All four were loaded as tool call 1 in 39 of 40 runs (call 2 once).
- Every guide's first sentence states what the guide does (a principle), and its "Use when" list
  is micro-activities plus symptoms. That family loads on greenfield (7 of 10 runs load four or
  more of them) and almost nowhere else, regardless of which micro-activity it names.
- Naming the task is not enough when it is buried: `rust-keep-it-simple` contains "Use when adding
  a feature" and loaded 0 of 6 on feature-add; the `b1` texts added "Also use when refactoring or
  cleaning up existing Rust code" and loaded 0 of 24 (6 changed guides x 4 probes) on the refactor prompt.
- Length does not separate loaders from non-loaders inside the guide family (318 to 459 chars all
  load on greenfield; the shortest description, `async-rust` at 142, never loads). Across families,
  every 100% description is shorter than every guide.
- A description scoped to a domain absent from the code (`async-rust`, `unsafe-rust-review`) never
  loads, even when the prompt says "Use the Rust skills available in this project" (0 of 6, v1
  `main` arm) and even under a router (the router body does not list them).

## 3. What the agent says around the load

In 54 of the 60 runs that made a Skill call, there is no assistant text and no readable thinking
before the first Skill call (2262 thinking blocks in all, only 116 with text). The decision is
silent. The few lines that touch it:

1. Run 8 (v1 `main`, prompt says use the skills; only `async-rust` and `unsafe-rust-review`
   available), thinking before tool 7, skip: "The skills file only covers async and unsafe code,
   which this crate barely uses".
2. p2-14 (hook, feature-add), after the Stop hook, skip: "I didn't load the `rust-*` skills for
   these small fixes."
3. p4-11 (Haiku, greenfield), right after loading four guides: "The skills are loaded. I'll start
   with the crate skeleton".
4. p3-02 (skills, refactor-small), after loading only `rust-keep-it-simple` at call 2: "The code is
   already in reasonable shape. I'll make a few targeted changes".
5. Run 7 (v1 branch, prompt says use the skills), tool 46, using a loaded skill: "Running the
   skill's smell searches to find real candidates."

What precedes the unaided loads is consistent: greenfield loads come at calls 2 to 8 right after
`Read SPEC.md` and one `Bash`; refactor-small loads come at call 2 right after one `Bash` that
lists or reads the crate; feature-add runs open with five or more `Bash` calls and never load.

## 4. Answers

**(a) `rust-keep-it-simple` on feature-add.** Confirmed for the description alone, with a
correction in scope. Feature-add runs where it was available: 9 (P2 `skills` 3, `hook` 3,
`router` 3). Description alone: 0 of 6 (`skills` 0/3, `hook` before its Stop hook 0/3). After the
Stop hook: 0 of 3 (the hook runs loaded `rust-test-quality` and `rust-error-discipline` instead,
matching the hook's findings). With the router: 3 of 3, at call 6, because the router body lists
it. So "no skill loaded on feature-add" (SUMMARY, `skills` arm 0 of 3) holds for the guides by
description; it is not true of feature-add overall: entry skills loaded 9 of 9 (`rust-code` 3,
`rust-rules` 6) and the hook arm loaded guides in 2 of 3 runs after its feedback.

**(b) Greenfield, five of six.** Three runs loaded exactly five of the six new guides: p1-12
(`skills`, Sonnet), p1-14 (`hook`, Sonnet, before the hook) and p4-11 (`skills`, Haiku). All three
skipped `rust-ownership`. The two Opus runs loaded four, skipping `rust-ownership` and
`rust-keep-it-simple`. `rust-ownership` loaded unaided in only 2 of 10 greenfield runs (p1-05,
p1-08, both at call 8, the last guide loaded).

**(c) Entry skills.** Each loaded in every run that had it, 40 of 40, at call 1 (39) or 2 (p1-09):
`rust-code` 9/9 and `rust-rules` 18/18 across greenfield, feature-add and refactor-small;
`rust-audit` 5/5 and `rust-refactor` 8/8, but those two were only offered on refactor tasks
(refactor-small, refactor-large, probe), so "every task type" is shown only for `rust-code` and
`rust-rules` (the `limits` arm shares `rust-rules`' text with `short`). Neither of those was offered
on the large crate. Common wording: request-shaped first sentence, enumeration of request kinds,
"Use first" plus a "before ..." anchor, under 300 characters, no micro-activities, no symptoms,
and (three of four) a promise of what it leads to ("It lists the other rust-* skills to load", "It
holds this project's Rust rules", "It says what to look for and which other rust-* skills to load").

**(d) Late loads.** Never without an outside push. Across all 116 runs, unaided loads sit at calls
1 to 8; router-driven loads end at call 8. The only Skill calls after call 8 are 9 calls in four
hook-arm runs (p1-02 at 20 to 22, p1-11 at 25 to 27, p2-04 at 18 to 19, p2-15 at 19), each the
call right after the Stop hook's feedback. The hook fired in all 9 hook-arm runs; in the other five
(p1-14, p2-14, p3-05, p3-06, p3-13) nothing was loaded after it.

## 5. Agreement with `v2/REPORT.md` section 6.1

Every row of the 6.1 table matches these counts (skill calls per run and first-call position for
all 18 rows), as do its claims that entry skills loaded in 30 v2 runs plus both P5 runs, that only
Stop-hook-requested calls come after call 8, and the per-guide greenfield counts. Two framing
notes, not count errors:

- 6.1 and SUMMARY say no branch run on the large crate loaded anything; that is true for the plain
  prompt (runs 1, 3; probes `a` 0/4) but runs 6 and 7, with the "Use the Rust skills" line, each
  loaded `rust-keep-it-simple` and `rust-comments` (calls 2 to 4); run 9 with the same line did not.
- SUMMARY's "Agregar feature: No (0 de 3)" is the `skills` arm only; the `hook` arm before its
  hook adds 0 of 3 more (0 of 6 by description).

## 6. What wording predicts loading

**Shown by the data:**

- A description whose first sentence restates the request and says "use first ... before ..."
  loaded in 40 of 40 runs on every task it was offered on, at call 1 or 2.
- Guides described by a principle plus micro-activities and symptoms load almost only on
  greenfield (7 of 10 runs, four to six guides each). On feature-add 0 of 6, on refactor of the
  large crate 0 of 8, on refactor of the small crate only `rust-keep-it-simple` (6 of 6 where no
  entry skill competed) and once `rust-comments`.
- Adding a task sentence at the end of a guide description ("Also use when refactoring ...") did
  not produce a single load in 4 probes. Naming "adding a feature" inside a list did not either
  (0 of 6).
- Domain-scoped descriptions (async, unsafe) never load when the code lacks that domain; the one
  recorded reason says so.
- Symptom-named feedback works where symptom-named descriptions do not: after the hook listed
  "stringly typed", "silent default", "weak assertion" lines, the agent loaded exactly the guides
  whose descriptions name those symptoms (`rust-domain-types`, `rust-error-discipline`,
  `rust-test-quality`), never `rust-keep-it-simple`, `rust-ownership` or `rust-comments`.
- A narrow prompt that names the topic ("Review the error handling") loaded the matching guide at
  call 2 (1 of 1).
- Loads happen in the first 8 calls or never, absent an external prompt.
- 35 built-in skills, 116 runs each, zero loads; their wording was not recorded.

**Inferred, not shown:**

- The agent matches descriptions once, against the request as a whole, after a first look at the
  inputs; descriptions that read as "this is your request" win, descriptions that read as "this
  helps with a step inside your request" only win when the request is to produce many such steps
  from nothing (greenfield).
- `rust-keep-it-simple` loads on the small-crate refactor probably because its name and first
  sentence ("Keep Rust code simple") echo the prompt ("hard to read", "clean it up"), not because of
  its "Use when" list; the same prompt on the large crate did not trigger it, so crate size or the
  first look at the code also matters (untested).
- The trailing "Also use when refactoring" failed likely because it sits at the end of a 400+ char
  micro-activity list; whether the same sentence as the first sentence would work was not tested.
- Five-of-six greenfield loads drop `rust-ownership` likely because its triggers (borrow checker
  error, `.clone()`, scan in a loop) describe a problem that has not happened yet in an empty crate.
- "Always activate" / "[Critical]" wording: no evidence either way in these logs.
