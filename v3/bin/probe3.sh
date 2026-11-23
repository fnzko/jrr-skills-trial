#!/bin/bash
# usage: probe3.sh <variant> <task: green|feature|refactor> <i>
# -> work dir /Users/Shared/jrr-work/v3-<variant>-<task>-<i>, log v3/logs/<variant>-<task>-<i>.jsonl
set -eu
R="/Users/facundo/code/wye/jrr-refactor-trial"; V2="$R/v2"; V3="$R/v3"
variant="$1"; task="$2"; i="$3"
id="$variant-$task-$i"
W="/Users/Shared/jrr-work/v3-$id"
plugin="$V3/variants/$variant"
[ -f "$plugin/plugin.json" ] || { echo "no plugin: $plugin" >&2; exit 1; }
if [ -e "$W" ]; then echo "work dir exists: $W" >&2; exit 1; fi
mkdir -p "$W" "$V3/logs"; cd "$W"
git init -q
if [ "$task" = green ]; then
  cp "$V2/task/SPEC.md" .
else
  cp -R "$V2/baseline"/. .
  [ -f SPEC.md ] || cp "$V2/task/SPEC.md" .
  [ "$task" = feature ] && cp "$V2/task/SPEC-feature.md" .
fi
printf 'target\n' > .gitignore
git add -A && git -c user.name=trial -c user.email=trial@example.invalid commit -qm "start" && git tag start
CLAUDE_CONFIG_DIR="$R/claude-home" timeout 300 claude -p \
  --plugin-dir "$plugin" \
  --model claude-sonnet-5-5 \
  --permission-mode auto --strict-mcp-config \
  --disallowedTools "WebFetch" "WebSearch" "Bash(git push:*)" \
  --max-turns 10 --max-budget-usd 0.5 \
  --output-format stream-json --verbose \
  "$(cat "$V2/prompts/$task.txt")" > "$V3/logs/$id.jsonl" 2> "$V3/logs/$id.err" || echo "claude exit $?" >> "$V3/logs/$id.err"
