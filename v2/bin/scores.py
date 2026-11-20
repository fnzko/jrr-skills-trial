"""usage: scores.py <phase>   joins blind dimension scores with the arm key; prints per-run and per-arm tables"""
import collections, os, statistics as st, sys
V = os.path.expanduser("~/code/wye/jrr-refactor-trial/v2")
ph = sys.argv[1]
code2run = dict(l.split() for l in open(f"{V}/keys/blind-{ph}.tsv") if l.strip())
arm = {}
for l in open(f"{V}/keys/runs.tsv"):
    p = l.rstrip("\n").split("\t")
    if len(p) >= 4:
        arm[p[0]] = p[1] + ("" if p[3].endswith("sonnet-5-5") else "/" + p[3].replace("claude-", "").split("-")[0])
dims = ["types", "errors", "ownership", "simplicity", "comments", "tests", "overall"]
score = collections.defaultdict(dict)
have = []
for d in dims:
    p = f"{V}/reviews/{ph}/{d}.tsv"
    if not os.path.exists(p):
        continue
    have.append(d)
    for l in open(p):
        f = l.split()
        if len(f) >= 2 and f[0] in code2run:
            score[code2run[f[0]]][d] = float(f[1])
        elif len(f) >= 2 and f[0] == "BASE":
            score["BASE"][d] = float(f[1])
print("\t".join(["run", "arm"] + have + ["sum6"]))
rows = []
for run in sorted(score):
    s = score[run]
    six = sum(s.get(d, 0) for d in have if d != "overall")
    rows.append((run, arm.get(run, "BASE"), s, six))
    print("\t".join([run, arm.get(run, "BASE")] + [str(s.get(d, "")) for d in have] + [str(six)]))
print()
by = collections.defaultdict(list)
for run, a, s, six in rows:
    by[a].append((s, six))
print("\t".join(["arm", "n"] + have + ["sum6", "sum6 values"]))
for a in sorted(by, key=lambda a: -st.mean(x[1] for x in by[a])):
    v = by[a]
    print("\t".join([a, str(len(v))] + [str(round(st.mean(x[0].get(d, 0) for x in v), 2)) for d in have] + [str(round(st.mean(x[1] for x in v), 2)), ",".join(str(x[1]) for x in v)]))
