#!/bin/bash
# usage: run2.sh <run> <arm> <task: green|feature|refactor> <model> [baseline-dir]
set -eu
R="$HOME/code/wye/jrr-refactor-trial"; V="$R/v2"
run="$1"; arm="$2"; task="$3"; model="$4"; base="${5:-}"
W="/Users/Shared/jrr-work/$run"
if [ -e "$W" ]; then echo "work dir exists: $W" >&2; exit 1; fi
mkdir -p "$W"; cd "$W"
git init -q
if [ "$task" = green ]; then
  cp "$V/task/SPEC.md" .
else
  cp -R "$base"/. .
  [ -f SPEC.md ] || cp "$V/task/SPEC.md" .
  [ "$task" = feature ] && cp "$V/task/SPEC-feature.md" .
fi
[ -f "$V/arms/$arm/CLAUDE.md" ] && cp "$V/arms/$arm/CLAUDE.md" .
printf 'target\n' > .gitignore
git add -A && git -c user.name=trial -c user.email=trial@example.invalid commit -qm "start" && git tag start
plugin=()
[ -f "$V/arms/$arm/plugin.json" ] && plugin=(--plugin-dir "$V/arms/$arm")
CLAUDE_CONFIG_DIR="$R/claude-home" timeout "${RUN_TIMEOUT:-4500}" claude -p \
  ${plugin[@]+"${plugin[@]}"} \
  --model "$model" \
  --permission-mode auto --strict-mcp-config \
  --disallowedTools "WebFetch" "WebSearch" "Bash(git push:*)" \
  --max-budget-usd "${BUDGET:-10}" \
  --output-format stream-json --verbose \
  "$(cat "$V/prompts/$task.txt")" > "$V/logs/$run.jsonl" 2> "$V/logs/$run.err" || echo "claude exit $?" >> "$V/logs/$run.err"
