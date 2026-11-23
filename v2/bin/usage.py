"""Consumption per run from stream-json logs.

usage: usage.py <keys.tsv> <logs-dir> [--runs]
keys.tsv columns: run, arm, task, model. Prints means by task and arm, or one line per run.
"""
import json
import sys
from collections import defaultdict
from pathlib import Path


def read_run(path):
    results = []
    with open(path, errors="replace") as handle:
        for line in handle:
            if '"type":"result"' not in line:
                continue
            try:
                message = json.loads(line)
            except json.JSONDecodeError:
                continue
            if message.get("type") == "result":
                results.append(message)
    if not results:
        return None
    tokens = defaultdict(int)
    for model_usage in results[-1].get("modelUsage", {}).values():
        tokens["input"] += model_usage.get("inputTokens", 0)
        tokens["cache_write"] += model_usage.get("cacheCreationInputTokens", 0)
        tokens["cache_read"] += model_usage.get("cacheReadInputTokens", 0)
        tokens["output"] += model_usage.get("outputTokens", 0)
    return {
        "results": len(results),
        "turns": sum(r.get("num_turns", 0) for r in results),
        "minutes": sum(r.get("duration_ms", 0) for r in results) / 60000,
        "api_minutes": sum(r.get("duration_api_ms", 0) for r in results) / 60000,
        "cost": results[-1].get("total_cost_usd", 0.0),
        "fresh_in": tokens["input"] + tokens["cache_write"],
        "cache_read": tokens["cache_read"],
        "output": tokens["output"],
    }


FIELDS = ["turns", "minutes", "cost", "fresh_in", "cache_read", "output"]


def main():
    keys, logs = Path(sys.argv[1]), Path(sys.argv[2])
    per_run = "--runs" in sys.argv
    seen = {}
    for line in keys.read_text().splitlines():
        parts = line.split("\t") if "\t" in line else line.split()
        if len(parts) < 4:
            continue
        seen[parts[0]] = (parts[1], parts[2], parts[3])
    cells = defaultdict(list)
    for run, (arm, task, model) in sorted(seen.items()):
        log = logs / f"{run}.jsonl"
        if not log.exists():
            continue
        data = read_run(log)
        if data is None:
            continue
        if per_run:
            print(run, arm, task, model, *(f"{data[f]:.2f}" if isinstance(data[f], float) else data[f] for f in ["results"] + FIELDS), sep="\t")
        cells[(task, model, arm)].append(data)
    if per_run:
        return
    print("task", "model", "arm", "n", *FIELDS, "cost_vs_none", "min_vs_none", sep="\t")
    base = {}
    for (task, model, arm), rows in cells.items():
        if arm == "none":
            base[(task, model)] = {f: sum(r[f] for r in rows) / len(rows) for f in FIELDS}
    for (task, model, arm), rows in sorted(cells.items()):
        mean = {f: sum(r[f] for r in rows) / len(rows) for f in FIELDS}
        ref = base.get((task, model))
        rel = [f"{mean['cost'] / ref['cost']:.2f}x", f"{mean['minutes'] / ref['minutes']:.2f}x"] if ref else ["-", "-"]
        print(task, model.replace("claude-", ""), arm, len(rows),
              f"{mean['turns']:.0f}", f"{mean['minutes']:.1f}", f"{mean['cost']:.2f}",
              f"{mean['fresh_in'] / 1000:.0f}k", f"{mean['cache_read'] / 1e6:.2f}M", f"{mean['output'] / 1000:.0f}k",
              *rel, sep="\t")


main()
