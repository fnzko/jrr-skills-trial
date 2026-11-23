"""Extract per-run skill availability, Skill calls and early reasoning.

Usage: python3 -I v3/descriptions/extract.py <trial-root> > runs.json
"""
import csv
import glob
import json
import os
import re
import sys

ROOT = sys.argv[1]


def v1_arm(run):
    rows = dict(l.rstrip("\n").split("\t", 1) for l in open(os.path.join(ROOT, "mapping.tsv")) if l.strip())
    return rows.get(run)


def v2_keys():
    keys = {}
    for row in csv.reader(open(os.path.join(ROOT, "v2/keys/runs.tsv")), delimiter="\t"):
        if row and row[0].startswith("p"):
            keys[row[0]] = row
    return keys


V2 = v2_keys()


def classify(path):
    name = os.path.basename(path)[:-6]
    if "/v2/" in path:
        row = V2[name]
        task = {"green": "greenfield", "feature": "feature-add", "refactor": "refactor-small"}[row[2]]
        return name, "v2", row[1], task
    if name.startswith("probe"):
        variant = name.split("-")[1] if "-" in name else "narrow-review"
        arm = {"a": "branch", "b1": "branch+b1", "b2": "branch+router-b2", "narrow-review": "branch"}[variant]
        return name, "v1", arm, "probe" if variant != "narrow-review" else "probe-review"
    if name == "pilot":
        return name, "v1", "pilot", "refactor-large"
    arm = v1_arm(name) or "?"
    short = arm
    if arm.startswith("branch+router (probe-plugins/b2"):
        short = "branch+router-b2"
    elif arm.startswith("branch+neutral"):
        short = "branch+router-b3"
    elif arm.startswith("branch+audit"):
        short = "audit"
    prompt = "prompt-skills" if name in {"4", "5", "6", "7", "8", "9"} else "plain"
    return name, "v1", f"{short}/{prompt}", "refactor-large"


def analyze(path):
    run, trial, arm, task = classify(path)
    inits = []
    tools = []
    seen = set()
    sub_skills = []
    early_text = []
    results = 0
    for line in open(path):
        try:
            m = json.loads(line)
        except ValueError:
            continue
        t = m.get("type")
        if t == "system" and m.get("subtype") == "init":
            inits.append({"model": m.get("model"), "skills": m.get("skills", []),
                          "plugins": [p.get("path") for p in m.get("plugins", []) if p.get("path") != "builtin"]})
        if t == "result":
            results += 1
        if t != "assistant":
            continue
        parent = m.get("parent_tool_use_id")
        for b in (m.get("message") or {}).get("content") or []:
            bt = b.get("type")
            if parent is None and bt in ("text", "thinking"):
                txt = (b.get("text") or b.get("thinking") or "").strip()
                if txt and len(tools) < 12:
                    early_text.append({"before_tool": len(tools) + 1, "kind": bt, "text": txt[:1500],
                                       "session": len(inits)})
            if bt != "tool_use" or b.get("id") in seen:
                continue
            seen.add(b.get("id"))
            if parent is not None:
                if b.get("name") == "Skill":
                    sub_skills.append((b.get("input") or {}).get("skill"))
                continue
            inp = b.get("input") or {}
            tools.append({"name": b.get("name"), "skill": inp.get("skill") if b.get("name") == "Skill" else None,
                          "session": len(inits),
                          "read_skill_md": b.get("name") == "Read" and "SKILL.md" in str(inp.get("file_path", ""))})
    avail = sorted(set(s for i in inits for s in i["skills"]))
    calls = [{"skill": x["skill"], "index": n + 1, "session": x["session"]}
             for n, x in enumerate(tools) if x["name"] == "Skill"]
    return {
        "run": run, "trial": trial, "arm": arm, "task": task,
        "model": inits[0]["model"] if inits else None,
        "n_inits": len(inits), "n_results": results, "plugins": sorted(set(p for i in inits for p in i["plugins"])),
        "available": avail, "n_tools": len(tools), "skill_calls": calls,
        "subagent_skill_calls": sub_skills,
        "skill_md_reads": sum(1 for x in tools if x["read_skill_md"]),
        "first_tools": [x["name"] + (":" + x["skill"] if x["skill"] else "") for x in tools[:10]],
        "early_text": early_text,
    }


paths = sorted(glob.glob(os.path.join(ROOT, "logs/*.jsonl")) + glob.glob(os.path.join(ROOT, "v2/logs/*.jsonl")))
json.dump([analyze(p) for p in paths], sys.stdout, indent=1)
