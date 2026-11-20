"""usage: table.py <phase-prefix> [cols...]  prints one row per run with its arm, from v2/eval/*/summary.json"""
import json, os, sys
V = os.path.expanduser("~/code/wye/jrr-refactor-trial/v2")
arms = {}
for l in open(f"{V}/keys/runs.tsv"):
    p = l.rstrip("\n").split("\t")
    if len(p) >= 4:
        arms[p[0]] = (p[1], p[3])
cols = sys.argv[2:] or ["acceptance", "own_tests", "skills_invoked", "hook_fired", "turns", "minutes", "cost_usd", "loc_prod", "newtypes", "enums", "stringly_ids_per_kloc", "str_compared_per_kloc", "unwrap_per_kloc", "expect_per_kloc", "unwrap_or_per_kloc", "clone_per_kloc", "clippy_per_kloc", "comment_pct", "tests", "weak_asserts"]
print("\t".join(["run", "arm", "model"] + cols))
for run in sorted(os.listdir(f"{V}/eval")):
    if not run.startswith(sys.argv[1]):
        continue
    try:
        d = json.load(open(f"{V}/eval/{run}/summary.json"))
    except (OSError, ValueError):
        continue
    a = arms.get(run, ("?", "?"))
    def fmt(v):
        if isinstance(v, list):
            return str(len(v))
        return str(v)
    print("\t".join([run, a[0], a[1].replace("claude-", "")] + [fmt(d.get(c)) for c in cols]))
