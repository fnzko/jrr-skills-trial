#!/bin/sh
# usage: eval.sh <run-name>      (run-name "baseline" evaluates the untouched clone)
# Writes eval/<run-name>/{commits,diffstat,test,clippy,metrics,transcript}.txt
set -eu
R="$HOME/code/wye/jrr-refactor-trial"
RESEARCH="$HOME/code/wye/research/rust-idiomatic-ai-patterns-2026-11-16"
CRATE=service-com-amazonaws-sqs-amazonsqs
name="$1"
if [ "$name" = baseline ]; then wt="$R/aws-emulator"; else wt="$R/runs/$name"; fi
out="$R/eval/$name"
mkdir -p "$out"
export CARGO_TARGET_DIR="${EVAL_TARGET:-$R/eval-target}"

cd "$wt"
git log --reverse --format='%h %s' baseline..HEAD > "$out/commits.txt"
git log --format='%b' baseline..HEAD | grep -c . > "$out/commit-body-lines.txt" || true
git diff --stat baseline..HEAD -- . ':!Cargo.lock' | tail -1 > "$out/diffstat.txt"
git diff --stat baseline..HEAD >> "$out/diffstat.txt"
git status --short > "$out/uncommitted.txt"

if cargo test -p "$CRATE" > "$out/test.log" 2>&1; then echo PASS; else echo FAIL; fi > "$out/test.txt"
grep -E '^test result' "$out/test.log" >> "$out/test.txt" || true

cargo clippy -p "$CRATE" --all-targets --message-format=json -- \
  -W clippy::cast_possible_truncation -W clippy::cast_possible_wrap \
  -W clippy::format_push_string -W clippy::manual_let_else \
  -W clippy::needless_for_each -W clippy::non_std_lazy_statics \
  -W clippy::or_fun_call -W clippy::allow_attributes \
  -W clippy::allow_attributes_without_reason -W clippy::dbg_macro \
  -W clippy::let_underscore_must_use -W clippy::unused_result_ok \
  -W clippy::wildcard_enum_match_arm 2> "$out/clippy.err" |
  python3 -I -c '
import json, sys, collections
c = collections.Counter()
for line in sys.stdin:
    try:
        m = json.loads(line)
    except ValueError:
        continue
    msg = m.get("message") or {}
    code = (msg.get("code") or {}).get("code")
    spans = msg.get("spans") or []
    if code and any("'"$CRATE"'" in s.get("file_name", "") for s in spans):
        c[code] += 1
print("total", sum(c.values()))
for k, v in sorted(c.items(), key=lambda kv: -kv[1]):
    print(f"{v:6d} {k}")
' > "$out/clippy.txt"

printf 'src\tsrc\t%s\ntests\ttests\t%s\n' "$wt/services/$CRATE" "$wt/services/$CRATE" > "$out/spec.tsv"
python3 -I "$RESEARCH/scripts/metrics.py" "$out/spec.tsv" > "$out/metrics.txt"

if [ -f "$R/logs/$name.jsonl" ]; then
  python3 -I "$R/transcript.py" "$R/logs/$name.jsonl" > "$out/transcript.txt"
fi
echo "done $name"
