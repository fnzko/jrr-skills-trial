#!/bin/sh
# Stop hook: run the just-right-rust detector once per session and hand the findings back.
input=$(cat)
sid=$(printf '%s' "$input" | python3 -I -c 'import json,sys; print(json.load(sys.stdin).get("session_id","x"))' 2>/dev/null)
cwd=$(printf '%s' "$input" | python3 -I -c 'import json,sys; print(json.load(sys.stdin).get("cwd","."))' 2>/dev/null)
marker="${TMPDIR:-/tmp}/jrr-hook-$sid"
[ -e "$marker" ] && exit 0
cd "$cwd" 2>/dev/null || exit 0
[ -f Cargo.toml ] || exit 0
: > "$marker"
out=$(mktemp)
cargo clippy --all-targets --message-format=short -- \
  -W clippy::cast_possible_truncation -W clippy::cast_possible_wrap \
  -W clippy::format_push_string -W clippy::manual_let_else \
  -W clippy::needless_for_each -W clippy::or_fun_call -W clippy::allow_attributes \
  -W clippy::dbg_macro -W clippy::let_underscore_must_use -W clippy::unused_result_ok \
  -W clippy::wildcard_enum_match_arm -W clippy::needless_pass_by_value \
  -W clippy::unwrap_used -W clippy::expect_used 2>&1 | grep -E '^[^ ]+\.rs:[0-9]+' | head -40 > "$out"
{
  grep -rnE '\b(\w+_)?(id|ids|name|key|url|code|handle): (Option<|Vec<)?(String|&(.\w+ )?str)\b|Map<String,' src 2>/dev/null | sed 's/^/stringly typed: /' | head -15
  grep -rnE 'match [^{;]*\.as_str\(\)|[!=]= "[A-Za-z]|"\w+" [!=]=' src 2>/dev/null | sed 's/^/string compared as a value: /' | head -15
  grep -rnE '\.unwrap_or_default\(\)|\.unwrap_or\((0|""|false|String::new\(\))\)|^\s*let _ = |\.ok\(\);' src 2>/dev/null | sed 's/^/silent default or dropped result: /' | head -15
  grep -rnE 'Result<[^;{}]*,\s*(String|&.static\s+str|\(\))\s*>|Box<dyn (std::error::)?Error' src 2>/dev/null | sed 's/^/untyped error: /' | head -10
  grep -rnE 'assert!\(\s*!?[^;]*\.is_(ok|err|some|none|empty)\(\)\s*[,)]|assert!\([^;]*\.contains\(' src tests 2>/dev/null | sed 's/^/weak assertion: /' | head -10
} >> "$out"
n=$(wc -l < "$out" | tr -d ' ')
if [ "$n" -gt 0 ]; then
  {
    echo "just-right-rust check: $n candidate findings (first lines shown)."
    cat "$out"
    echo "For each one, fix it or say in one line why it stays. Load the matching rust-* skill for how to fix. Then finish."
  } >&2
  exit 2
fi
exit 0
