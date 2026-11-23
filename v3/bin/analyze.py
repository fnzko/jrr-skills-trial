"""Summarize v3 probe logs.

Usage: python3 -I v3/bin/analyze.py [log.jsonl ...]   (default: all of v3/logs)
Prints one line per probe, then a variant x task table of skill load counts.
"""
import collections
import json
import pathlib
import sys

LOGS = pathlib.Path("/Users/facundo/code/wye/jrr-refactor-trial/v3/logs")
TASK_SKILL = {"green": "rust-new-code", "feature": "rust-add-feature", "refactor": "rust-refactor"}
TASK_SKILLS = set(TASK_SKILL.values())
SIX = ["rust-comments", "rust-domain-types", "rust-error-discipline", "rust-keep-it-simple", "rust-ownership", "rust-test-quality"]
ABBR = {
    "rust-comments": "com", "rust-domain-types": "dom", "rust-error-discipline": "err",
    "rust-keep-it-simple": "kis", "rust-ownership": "own", "rust-test-quality": "tst",
    "rust-new-code": "NEW", "rust-refactor": "REF", "rust-add-feature": "ADD",
    "rust-rules": "rules", "async-rust": "async", "unsafe-rust-review": "unsafe",
}


def parse(path):
    p = {"id": path.stem, "skills": [], "tool_count": 0, "cost": None, "offered": [], "auth_error": False, "subtype": None}
    for line in path.read_text(errors="replace").splitlines():
        try:
            m = json.loads(line)
        except ValueError:
            continue
        if "Not logged in" in line or "Invalid API key" in line:
            p["auth_error"] = True
        t = m.get("type")
        if t == "system" and m.get("subtype") == "init":
            p["offered"] = [s for s in m.get("skills", []) if s.startswith("just-right-rust:")]
        elif t == "result":
            p["cost"] = m.get("total_cost_usd")
            p["subtype"] = m.get("subtype")
        elif t == "assistant":
            for b in (m.get("message") or {}).get("content") or []:
                if b.get("type") != "tool_use":
                    continue
                p["tool_count"] += 1
                if b.get("name") == "Skill":
                    s = str((b.get("input") or {}).get("skill", "")).split(":")[-1]
                    p["skills"].append((s, p["tool_count"]))
    variant, task, i = p["id"].split("-")
    p.update(variant=variant, task=task, i=i)
    return p


def main():
    paths = [pathlib.Path(a) for a in sys.argv[1:]] or sorted(LOGS.glob("*.jsonl"))
    probes = [parse(x) for x in paths]
    total = 0.0
    print("| probe | Skill calls (skill@tool index) | tools | offered | cost | end |")
    print("|---|---|---|---|---|---|")
    for p in probes:
        total += p["cost"] or 0
        sk = ", ".join(f"{ABBR.get(s, s)}@{n}" for s, n in p["skills"]) or "-"
        flag = " AUTH-ERROR" if p["auth_error"] else ""
        cost = f"{p['cost']:.3f}" if p["cost"] is not None else "?"
        print(f"| {p['id']} | {sk} | {p['tool_count']} | {len(p['offered'])} | {cost} | {p['subtype']}{flag} |")
    print(f"\nprobes {len(probes)}, total cost ${total:.2f}\n")

    cells = collections.defaultdict(list)
    for p in probes:
        cells[(p["variant"], p["task"])].append(p)
    print("| variant | task | n | loads per skill (runs of n) | first Skill call index | cost/probe |")
    print("|---|---|---|---|---|---|")
    for (v, t), ps in sorted(cells.items()):
        c = collections.Counter()
        for p in ps:
            for s in {s for s, _ in p["skills"]}:
                c[s] += 1
        loads = " ".join(f"{ABBR.get(s, s)} {k}" for s, k in sorted(c.items(), key=lambda x: (-x[1], x[0]))) or "none"
        firsts = ",".join(str(p["skills"][0][1]) if p["skills"] else "-" for p in ps)
        six = sum(1 for p in ps for s in {s for s, _ in p["skills"]} if s in SIX)
        extra = ""
        if v in ("v4", "v5"):
            right = sum(1 for p in ps if TASK_SKILL[t] in {s for s, _ in p["skills"]})
            wrong = sum(1 for p in ps if ({s for s, _ in p["skills"]} & TASK_SKILLS) - {TASK_SKILL[t]})
            extra = f"; matching task skill {right}/{len(ps)}, wrong task skill {wrong}/{len(ps)}"
        cost = sum(p["cost"] or 0 for p in ps) / len(ps)
        print(f"| {v} | {t} | {len(ps)} | {loads}{extra} (six-total {six}) | {firsts} | {cost:.3f} |")


if __name__ == "__main__":
    main()
