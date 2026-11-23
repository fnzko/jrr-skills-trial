"""Stop hook: check the Rust lines this session added and hand the findings back once."""
import json
import os
import re
import subprocess
import sys
import tempfile
from collections import defaultdict
from pathlib import Path

LINTS = [
    "cast_possible_truncation", "cast_possible_wrap", "format_push_string", "manual_let_else",
    "needless_for_each", "or_fun_call", "allow_attributes", "dbg_macro", "let_underscore_must_use",
    "unused_result_ok", "wildcard_enum_match_arm", "needless_pass_by_value", "unwrap_used", "expect_used",
]
PATTERNS = [
    ("stringly typed", r'\b(\w+_)?(id|ids|name|key|url|code|handle): (Option<|Vec<)?(String|&(.\w+ )?str)\b|Map<String,'),
    ("string compared as a value", r'match [^{;]*\.as_str\(\)|[!=]= "[A-Za-z]|"\w+" [!=]='),
    ("silent default or dropped result", r'\.unwrap_or_default\(\)|\.unwrap_or\((0|""|false|String::new\(\))\)|^\s*let _ = |\.ok\(\);'),
    ("untyped error", r'Result<[^;{}]*,\s*(String|&.static\s+str|\(\))\s*>|Box<dyn (std::error::)?Error'),
    ("weak assertion", r'assert!\(\s*!?[^;]*\.is_(ok|err|some|none|empty)\(\)\s*[,)]|assert!\([^;]*\.contains\('),
]


def git(cwd, *args):
    return subprocess.run(["git", "-C", cwd, *args], capture_output=True, text=True).stdout


def added_lines(cwd, base):
    added = defaultdict(set)
    path = None
    for line in git(cwd, "diff", "-U0", base, "--", "*.rs").splitlines():
        if line.startswith("+++ "):
            path = line[6:] if line.startswith("+++ b/") else None
        elif line.startswith("@@") and path:
            match = re.search(r"\+(\d+)(?:,(\d+))?", line)
            start, count = int(match.group(1)), int(match.group(2) or 1)
            added[path].update(range(start, start + count))
    for path in git(cwd, "ls-files", "--others", "--exclude-standard", "--", "*.rs").splitlines():
        try:
            total = len(Path(cwd, path).read_text(errors="replace").splitlines())
        except OSError:
            continue
        added[path].update(range(1, total + 1))
    return added


def main():
    event = json.load(sys.stdin)
    session, cwd = event.get("session_id", "x"), event.get("cwd", ".")
    tmp = Path(os.environ.get("TMPDIR") or tempfile.gettempdir())
    base_file, marker = tmp / f"jrr-base-{session}", tmp / f"jrr-stop-{session}"
    if marker.exists() or not base_file.exists() or not Path(cwd, "Cargo.toml").exists():
        return 0
    marker.touch()
    base = base_file.read_text().strip()
    if not base:
        return 0
    added = added_lines(cwd, base)
    if not added:
        return 0
    findings = []
    clippy = subprocess.run(
        ["cargo", "clippy", "--all-targets", "--message-format=short", "--"] + [f"-Wclippy::{lint}" for lint in LINTS],
        cwd=cwd, capture_output=True, text=True,
    )
    for line in clippy.stderr.splitlines():
        match = re.match(r"([^ :]+\.rs):(\d+)", line)
        if match and int(match.group(2)) in added.get(match.group(1), ()):
            findings.append(line)
    findings = findings[:40]
    for path, lines in sorted(added.items()):
        try:
            text = Path(cwd, path).read_text(errors="replace").splitlines()
        except OSError:
            continue
        for label, pattern in PATTERNS:
            hits = [n for n in sorted(lines) if n <= len(text) and re.search(pattern, text[n - 1])]
            findings += [f"{label}: {path}:{n}: {text[n - 1].strip()}" for n in hits[:10]]
    if not findings:
        return 0
    print(f"just-right-rust check: {len(findings)} candidate findings in the Rust lines this session added.", file=sys.stderr)
    print("\n".join(findings), file=sys.stderr)
    print("For each one, fix it or say in one line why it stays. Then finish.", file=sys.stderr)
    return 2


sys.exit(main())
