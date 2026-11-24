#!/bin/sh
# usage: rules.sh <start|context|gate>   Claude Code hook that hands the Rust checklist to the agent.
mode="$1"
dir=$(dirname "$0")
input=$(cat)
field() { printf '%s' "$input" | python3 -I -c "import json,sys; d=json.load(sys.stdin); print($1)" 2>/dev/null; }
sid=$(field 'd.get("session_id","x")')
if [ "$mode" = start ]; then
  base="${TMPDIR:-/tmp}/jrr-base-$sid"
  [ -e "$base" ] || git -C "$(field 'd.get("cwd",".")')" rev-parse HEAD > "$base" 2>/dev/null
  cat "$dir/scope.md" "$dir/checklist.md"
  exit 0
fi
path=$(field 'd.get("tool_input",{}).get("file_path","")')
case "$path" in *.rs) ;; *) exit 0 ;; esac
marker="${TMPDIR:-/tmp}/jrr-rules-$sid"
[ -e "$marker" ] && exit 0
: > "$marker"
if [ "$mode" = gate ]; then
  {
    echo "Before writing Rust in this project, read these rules. Then make the edit again, following them."
    echo
    cat "$dir/checklist.md"
  } >&2
  exit 2
fi
python3 -I -c 'import json,sys; print(json.dumps({"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":open(sys.argv[1]).read()}}))' "$dir/checklist.md"
