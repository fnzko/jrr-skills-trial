#!/bin/sh
# usage: rules.sh <context|gate>   PreToolUse hook: hands the Rust checklist to the agent before its first Rust write.
mode="$1"
dir=$(dirname "$0")
sid=$(python3 -I "$dir/writes_rust.py" 2>/dev/null)
[ -n "$sid" ] || exit 0
marker="${TMPDIR:-/tmp}/jrr-rules-$sid"
[ -e "$marker" ] && exit 0
: > "$marker"
if [ "$mode" = gate ]; then
  {
    echo "Before writing Rust in this project, read these rules. Then do this step again, following them."
    echo
    cat "$dir/checklist.md"
  } >&2
  exit 2
fi
python3 -I -c 'import json,sys; print(json.dumps({"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":open(sys.argv[1]).read()}}))' "$dir/checklist.md"
