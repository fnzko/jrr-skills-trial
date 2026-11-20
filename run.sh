#!/bin/sh
# usage: MODEL=<model> BUDGET=<usd> run.sh <run-name> <home: pilot|branch|main>
set -eu
R="$HOME/code/wye/jrr-refactor-trial"
name="$1"
home="$2"
wt="$R/runs/$name"

if [ ! -d "$wt" ]; then
  git -C "$R/aws-emulator" worktree add -q "$wt" -b "run-$name" baseline
fi
cd "$wt"
SYMPOSIUM_HOME="$R/homes/$home" "$R/tools/bin/cargo-agents" agents sync -q

CLAUDE_CONFIG_DIR="$R/claude-home" claude -p \
  --plugin-dir "${PLUGIN_DIR:-$wt/.claude/skills/just-right-rust}" \
  --model "$MODEL" \
  --permission-mode "${PERMISSION_MODE:-auto}" \
  --strict-mcp-config \
  --disallowedTools "WebFetch" "WebSearch" "Bash(git push:*)" \
  --max-budget-usd "$BUDGET" \
  --output-format stream-json --verbose \
  "$(cat "$R/${PROMPT:-prompt.txt}")" > "$R/logs/$name.jsonl" 2> "$R/logs/$name.err"
