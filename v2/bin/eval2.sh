#!/bin/bash
# usage: eval2.sh <run> <task>   -> v2/eval/<run>/summary.json, copies the crate to v2/runs/<run>
set -u
R="$HOME/code/wye/jrr-refactor-trial"; V="$R/v2"
run="$1"; task="$2"
W="/Users/Shared/jrr-work/$run"; out="$V/eval/$run"; mkdir -p "$out"
cd "$W" || exit 1
git log --reverse --format='%h %s' start..HEAD > "$out/commits.txt" 2>/dev/null
git diff --shortstat start..HEAD -- . ':!Cargo.lock' > "$out/diffstat.txt" 2>/dev/null
git status --short > "$out/uncommitted.txt"
if timeout 900 cargo build > "$out/build.log" 2>&1; then echo ok; else echo fail; fi > "$out/build.txt"
if timeout 900 cargo test > "$out/test.log" 2>&1; then echo ok; else echo fail; fi > "$out/test.txt"
feat=""; [ "$task" = feature ] && feat="--feature"
if [ -x target/debug/miniq ]; then
  timeout 900 python3 -I "$V/task/acceptance.py" "$W/target/debug/miniq" $feat --json > "$out/acceptance.json" 2> "$out/acceptance.err"
else
  echo '{"passed": 0, "total": 0, "failed": ["no binary"]}' > "$out/acceptance.json"
fi
timeout 900 cargo clippy --all-targets --message-format=json -- \
  -W clippy::cast_possible_truncation -W clippy::cast_possible_wrap \
  -W clippy::format_push_string -W clippy::manual_let_else \
  -W clippy::needless_for_each -W clippy::non_std_lazy_statics \
  -W clippy::or_fun_call -W clippy::allow_attributes \
  -W clippy::allow_attributes_without_reason -W clippy::dbg_macro \
  -W clippy::let_underscore_must_use -W clippy::unused_result_ok \
  -W clippy::wildcard_enum_match_arm > "$out/clippy.jsonl" 2> "$out/clippy.err"
python3 -I "$V/bin/detect.py" "$W" "$out" "$V/logs/$run.jsonl" > "$out/summary.json"
mkdir -p "$V/runs/$run"
rsync -a --exclude target --exclude .git "$W"/ "$V/runs/$run"/
git -C "$W" bundle create "$V/runs/$run/history.bundle" --all > /dev/null 2>&1
( cd "$W" && cargo clean > /dev/null 2>&1 )
echo "done $run $(python3 -I -c 'import json,sys; d=json.load(open(sys.argv[1])); print(d.get("acceptance"), d.get("skills_invoked"), d.get("turns"), d.get("minutes"))' "$out/summary.json")"
