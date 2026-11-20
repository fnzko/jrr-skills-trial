#!/bin/sh
# usage: probe.sh <variant: a|b1|b2> <n>   -> logs/probe-<variant>-<i>.jsonl
set -eu
R="$HOME/code/wye/jrr-refactor-trial"
v="$1"; n="$2"
cd "$R/runs/probe"
i=1
while [ "$i" -le "$n" ]; do
  git checkout -q . && git clean -fdq
  CLAUDE_CONFIG_DIR="$R/claude-home" claude -p \
    --plugin-dir "$R/probe-plugins/$v" \
    --model "${MODEL:-claude-sonnet-5-5}" \
    --permission-mode auto --strict-mcp-config \
    --disallowedTools "WebFetch" "WebSearch" "Bash(git push:*)" "Bash(cargo:*)" \
    --max-turns "${TURNS:-8}" \
    --output-format stream-json --verbose \
    "$(cat "$R/${PROMPT:-prompt.txt}")" > "$R/logs/probe-$v-$i.jsonl" 2> "$R/logs/probe-$v-$i.err" || true
  echo "$v $i $(python3 -I "$R/transcript.py" "$R/logs/probe-$v-$i.jsonl" | grep '^skills invoked')"
  i=$((i+1))
done
