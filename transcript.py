"""Summarize a `claude -p --output-format stream-json` transcript.

Usage: python3 -I transcript.py <log.jsonl>
"""
import collections
import json
import sys

tools = collections.Counter()
skills = []
skill_reads = collections.Counter()
bash = collections.Counter()
results = []
init = None

with open(sys.argv[1]) as fh:
    for line in fh:
        try:
            m = json.loads(line)
        except ValueError:
            continue
        if m.get("type") == "system" and m.get("subtype") == "init":
            init = m
        if m.get("type") == "result":
            results.append(m)
        if m.get("type") != "assistant":
            continue
        for block in (m.get("message") or {}).get("content") or []:
            if block.get("type") != "tool_use":
                continue
            name = block.get("name")
            inp = block.get("input") or {}
            tools[name] += 1
            if name == "Skill":
                skills.append(inp.get("skill"))
            if name == "Read" and "SKILL.md" in str(inp.get("file_path", "")):
                skill_reads[inp["file_path"].split("/skills/")[-1]] += 1
            if name == "Bash":
                cmd = str(inp.get("command", "")).strip()
                for key in ("cargo test", "cargo clippy", "cargo check", "cargo build", "cargo fmt", "git commit"):
                    if key in cmd:
                        bash[key] += 1

if init:
    print("model", init.get("model"))
    print("skills offered", [s for s in init.get("skills", []) if ":" in s])
print("skills invoked", skills)
print("skill files read", dict(skill_reads))
print("tool calls", dict(tools.most_common()))
print("bash", dict(bash))
if results:
    last = results[-1]
    print("result messages", len(results))
    print("subtype", last.get("subtype"))
    print("is_error", last.get("is_error"))
    print("num_turns", sum(r.get("num_turns") or 0 for r in results))
    print("duration_ms", sum(r.get("duration_ms") or 0 for r in results))
    print("total_cost_usd", last.get("total_cost_usd"))
    print("final message:")
    print(last.get("result"))
else:
    print("no result message (run still going or killed)")
