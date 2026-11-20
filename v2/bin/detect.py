"""Mechanical measures for one run. usage: detect.py <crate-dir> <eval-dir> <log.jsonl>  -> JSON on stdout"""
import collections, json, os, re, sys

crate, out, log = sys.argv[1], sys.argv[2], sys.argv[3]

def read(p):
    try:
        return open(p, errors="replace").read()
    except OSError:
        return ""

prod, test = [], []
for root in ("src", "tests", "benches"):
    for dp, _, fns in os.walk(os.path.join(crate, root)):
        for fn in fns:
            if not fn.endswith(".rs"):
                continue
            text = read(os.path.join(dp, fn))
            if root != "src":
                test.append(text)
                continue
            cut = re.search(r"(?m)^\s*#\[cfg\(test\)\]", text)
            if cut:
                prod.append(text[: cut.start()])
                test.append(text[cut.start():])
            else:
                prod.append(text)
P, T = "\n".join(prod), "\n".join(test)

def lines(s):
    return [l for l in s.splitlines() if l.strip()]

def n(pat, s, flags=re.M):
    return len(re.findall(pat, s, flags))

pl, tl = lines(P), lines(T)
comment = sum(1 for l in pl if l.strip().startswith("//"))
code = max(len(pl) - comment, 1)
m = {
    "loc_prod": len(pl), "loc_test": len(tl), "code_prod": code,
    "comment_pct": round(100.0 * comment / max(len(pl), 1), 1),
    "src_files": sum(len([f for f in fns if f.endswith(".rs")]) for _, _, fns in os.walk(os.path.join(crate, "src"))),
    "newtypes": n(r"^\s*(pub(\([\w:]+\))? )?struct \w+\s*\((pub(\([\w:]+\))? )?[\w:<>&' ]+\)\s*;", P),
    "enums": n(r"^\s*(pub(\([\w:]+\))? )?enum \w+", P),
    "structs": n(r"^\s*(pub(\([\w:]+\))? )?struct \w+", P),
    "stringly_ids": n(r"\b(\w+_)?(id|ids|name|key|url|code|handle|group|token): (Option<|Vec<)?(String|&('\w+ )?str)\b", P),
    "string_keyed_maps": n(r"Map<String,", P),
    "str_compared": n(r"match [^{;]*\.as_str\(\)|[!=]= \"[A-Za-z]|\"\w+\" [!=]=", P),
    "json_value_use": n(r"serde_json::Value|\bValue::", P),
    "unwrap": n(r"\.unwrap\(\)", P), "expect": n(r"\.expect\(", P),
    "unwrap_or": n(r"\.unwrap_or(_default|_else)?\(", P),
    "dropped_result": n(r"^\s*let _ = |\.ok\(\);", P),
    "panics": n(r"\b(panic|unreachable|todo|unimplemented)!\(", P),
    "string_errors": n(r"Result<[^;{}]*,\s*(String|&'static\s+str|\(\))\s*>|Box<dyn (std::error::)?Error", P),
    "as_casts": n(r"\bas (u|i)(8|16|32|64|128|size)\b", P),
    "clone": n(r"\.clone\(\)", P), "to_string": n(r"\.to_string\(\)|\.to_owned\(\)|String::from\(", P),
    "allow_attrs": n(r"#!?\[allow\(", P + T),
    "pub_fields": n(r"^\s+pub \w+: ", P), "bool_fields": n(r"^\s+(pub(\([\w:]+\))? )?\w+: bool,", P),
    "option_fields": n(r"^\s+(pub(\([\w:]+\))? )?\w+: Option<", P),
    "tests": n(r"#\[test\]", T), "asserts": n(r"\bassert(_eq|_ne)?!\(", T),
    "weak_asserts": n(r"assert!\(\s*!?[^;]*\.is_(ok|err|some|none|empty)\(\)\s*[,)]|assert!\([^;]*\.contains\(", T),
}
k = code / 1000.0
for key in ("stringly_ids", "str_compared", "unwrap", "expect", "unwrap_or", "clone", "to_string", "as_casts"):
    m[key + "_per_kloc"] = round(m[key] / k, 2)

lint = collections.Counter()
for line in read(os.path.join(out, "clippy.jsonl")).splitlines():
    try:
        d = json.loads(line)
    except ValueError:
        continue
    msg = d.get("message") or {}
    codeid = (msg.get("code") or {}).get("code")
    if codeid and msg.get("spans"):
        lint[codeid] += 1
m["clippy_total"] = sum(lint.values())
m["clippy_per_kloc"] = round(m["clippy_total"] / k, 2)
m["clippy_top"] = dict(lint.most_common(6))

try:
    acc = json.loads(read(os.path.join(out, "acceptance.json")) or "{}")
except ValueError:
    acc = {}
m["acceptance"] = f'{acc.get("passed", 0)}/{acc.get("total", 0)}'
m["acceptance_rate"] = round(acc.get("passed", 0) / acc["total"], 3) if acc.get("total") else 0.0
m["build"] = read(os.path.join(out, "build.txt")).strip()
m["own_tests"] = read(os.path.join(out, "test.txt")).strip()
m["commits"] = len(lines(read(os.path.join(out, "commits.txt"))))
m["diffstat"] = read(os.path.join(out, "diffstat.txt")).strip()

skills, results, tools, first_skill, calls, hook_fired = [], [], 0, None, 0, 0
model = None
for line in read(log).splitlines():
    try:
        d = json.loads(line)
    except ValueError:
        continue
    if d.get("type") == "system" and d.get("subtype") == "init":
        model = d.get("model")
    if "just-right-rust check:" in line:
        hook_fired += 1
    if d.get("type") == "result":
        results.append(d)
    if d.get("type") != "assistant":
        continue
    for b in (d.get("message") or {}).get("content") or []:
        if b.get("type") != "tool_use":
            continue
        calls += 1
        if b.get("name") == "Skill":
            skills.append(str((b.get("input") or {}).get("skill", "")).split(":")[-1])
            if first_skill is None:
                first_skill = calls
m.update({
    "model": model, "skills_invoked": skills, "first_skill_call": first_skill, "tool_calls": calls,
    "hook_fired": hook_fired,
    "turns": sum(r.get("num_turns") or 0 for r in results),
    "minutes": round(sum(r.get("duration_ms") or 0 for r in results) / 60000.0, 1),
    "cost_usd": round((results[-1].get("total_cost_usd") or 0), 2) if results else None,
    "is_error": results[-1].get("is_error") if results else True,
    "final": (str(results[-1].get("result"))[:300] if results else "no result message"),
})
print(json.dumps(m, indent=1))
