"""Reads a PreToolUse event on stdin and prints its session id if the call is about to write Rust."""
import json
import re
import sys

WRITES = r">>?|<<|\btee\b|sed -i|perl -pi|open\(|write_text|apply_patch"

event = json.load(sys.stdin)
tool, args = event.get("tool_name", ""), event.get("tool_input", {})
if tool == "Bash":
    command = args.get("command", "")
    hit = bool(re.search(r"\bcargo (init|new)\b", command)) or (".rs" in command and bool(re.search(WRITES, command)))
else:
    hit = str(args.get("file_path", "")).endswith(".rs")
if hit:
    print(event.get("session_id", "x"))
