"""Build the description x task x model tables from extract.py output.

Usage: python3 -I v3/descriptions/tables.py <runs.json> <trial-root>
"""
import collections
import glob
import json
import os
import re
import sys

runs = json.load(open(sys.argv[1]))
ROOT = sys.argv[2]

ROUTER_ARMS = ("router", "branch+router-b2", "branch+router-b3")
ENTRY = {"rust-code", "rust-audit", "rust-rules", "rust-refactor"}


def plugin_dir(path):
    return path.split("/.claude/skills/just-right-rust")[0] + "/.claude/skills/just-right-rust" if "/.claude/" in path else path


def descriptions(plugin):
    out = {}
    for f in glob.glob(os.path.join(plugin, "skills/*/SKILL.md")):
        fm = open(f).read().split("---")[1]
        m = re.search(r"^description:\s*(.*?)(?=^\w[\w-]*:|\Z)", fm, re.S | re.M)
        out[f.split("/")[-2]] = m.group(1).strip()
    return out


variant_ids = {}


def variant(name, desc):
    key = (name, desc)
    if key not in variant_ids:
        n = sum(1 for k in variant_ids if k[0] == name)
        variant_ids[key] = name + ("" if n == 0 else f" (v{n + 1})")
    return variant_ids[key]


def hook_feedback_index(run):
    path = os.path.join(ROOT, "v2/logs" if run["trial"] == "v2" else "logs", run["run"] + ".jsonl")
    n = 0
    for line in open(path):
        try:
            m = json.loads(line)
        except ValueError:
            continue
        if m.get("type") != "assistant" and "just-right-rust check:" in line:
            return n
        if m.get("type") == "assistant" and not m.get("parent_tool_use_id"):
            n += sum(1 for b in m["message"]["content"] if b.get("type") == "tool_use")
    return None


rows = collections.defaultdict(lambda: {"avail": 0, "avail_free": 0, "load_free": 0, "load_router": 0,
                                        "load_hook": 0, "load_promptline": 0, "avail_router": 0, "avail_hook": 0, "avail_promptline": 0, "pos": [], "runs": []})
builtin = collections.Counter()
builtin_loaded = collections.Counter()
desc_text = {}
model_short = lambda m: m.replace("claude-", "").replace("-5-5", "")

for r in runs:
    plugins = r["plugins"]
    descs = descriptions(plugin_dir(plugins[0])) if plugins else {}
    hook_at = hook_feedback_index(r) if r["arm"] == "hook" else None
    loaded = {}
    for c in r["skill_calls"]:
        loaded.setdefault(c["skill"].split(":")[-1], c["index"])
    router = r["arm"].split("/")[0] in ROUTER_ARMS
    promptline = r["arm"].endswith("/prompt-skills")
    for s in r["available"]:
        if not s.startswith("just-right-rust:"):
            builtin[s] += 1
            if any(c["skill"] == s for c in r["skill_calls"]):
                builtin_loaded[s] += 1
            continue
        name = s.split(":")[1]
        vid = variant(name, descs[name])
        desc_text[vid] = descs[name]
        key = (vid, r["task"], model_short(r["model"]))
        row = rows[key]
        row["avail"] += 1
        row["runs"].append(r["run"])
        free = not promptline and (not router or name in ENTRY)
        if free:
            row["avail_free"] += 1
        if router and name not in ENTRY:
            row["avail_router"] += 1
        if hook_at is not None:
            row["avail_hook"] += 1
        if promptline:
            row["avail_promptline"] += 1
        if name in loaded:
            idx = loaded[name]
            row["pos"].append(idx)
            if router and name not in ENTRY:
                row["load_router"] += 1
            elif hook_at is not None and idx > hook_at:
                row["load_hook"] += 1
            elif promptline:
                row["load_promptline"] += 1
            else:
                row["load_free"] += 1

def frac(row, k):
    return f"{row['load_' + k]} of {row['avail_' + k]}" if row["avail_" + k] else "-"


order = ["greenfield", "feature-add", "refactor-small", "refactor-large", "probe", "probe-review"]
print("| Description | Task | Model | Available | Loaded, description alone | Loaded, router body asked | Loaded, after Stop hook | Loaded, prompt said use skills | Position of Skill call |")
print("|---|---|---|---|---|---|---|---|---|")
for key in sorted(rows, key=lambda k: (k[0], order.index(k[1]), k[2])):
    row = rows[key]
    pos = ", ".join(str(p) for p in sorted(row["pos"])) or "-"
    print(f"| `{key[0]}` | {key[1]} | {key[2]} | {row['avail']} | {row['load_free']} of {row['avail_free']} | "
          f"{frac(row, 'router')} | {frac(row, 'hook')} | {frac(row, 'promptline')} | {pos} |")
print()
print("Built-in skills (descriptions not recorded in the transcripts):")
for s in sorted(builtin):
    print(f"- `{s}`: available {builtin[s]}, loaded {builtin_loaded[s]}")
print()
for vid, d in sorted(desc_text.items()):
    print(f"DESC\t{vid}\t{len(d)}\t{d}")
