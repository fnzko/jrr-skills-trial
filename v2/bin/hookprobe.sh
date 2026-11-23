#!/bin/bash
# usage: hookprobe.sh <arm>   one short run that shows whether the arm's hooks reach the agent
set -eu
R="$HOME/code/wye/jrr-refactor-trial"; V="$R/v2"; arm="$1"
W="/Users/Shared/jrr-work/hkprobe-$arm-$$"
mkdir -p "$W" "$V/logs/hookprobe"; cd "$W"; git init -q
printf 'target\n' > .gitignore
git add -A && git -c user.name=trial -c user.email=trial@example.invalid commit -qm start
CLAUDE_CONFIG_DIR="$R/claude-home" timeout 400 claude -p --plugin-dir "$V/arms/$arm" \
  --model claude-sonnet-5-5 --permission-mode auto --strict-mcp-config \
  --disallowedTools "WebFetch" "WebSearch" "Bash(git push:*)" \
  --max-turns 12 --max-budget-usd 0.6 --output-format stream-json --verbose \
  "Create a Rust library crate here by hand (Cargo.toml and src/lib.rs) with one function: pub fn parse_port(input: String) -> u16 that parses the text and uses expect on failure. Do not run cargo. When you are done, reply with the exact first heading line of any Rust rules you were shown during this session, or NONE if you saw none." \
  > "$V/logs/hookprobe/$arm.jsonl" 2> "$V/logs/hookprobe/$arm.err" || true
python3 -I - "$V/logs/hookprobe/$arm.jsonl" <<'PY'
import json, sys
hooks, final, denied = [], "", 0
for line in open(sys.argv[1], errors="replace"):
    try: m = json.loads(line)
    except ValueError: continue
    if m.get("type") == "system" and "hook" in str(m.get("subtype", "")):
        hooks.append((m.get("subtype"), m.get("hook_event") or m.get("hook_name")))
    if m.get("type") == "user":
        for c in m.get("message", {}).get("content", []) if isinstance(m.get("message", {}).get("content"), list) else []:
            if c.get("type") == "tool_result" and "Before writing Rust" in json.dumps(c): denied += 1
    if m.get("type") == "result": final = (m.get("result") or "")[-200:], m.get("num_turns"), round(m.get("total_cost_usd", 0), 3)
print("hook events:", sorted(set(hooks)))
print("gate denials seen:", denied)
print("final:", final)
PY
