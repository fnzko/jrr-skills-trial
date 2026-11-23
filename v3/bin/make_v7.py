"""Build v3/variants/v7: v2/arms/skills with the six descriptions from
v3/descriptions/guidance.md, section "Corrected descriptions", verbatim.

Usage: python3 -I v3/bin/make_v7.py
"""
import re
import shutil

from make_variants import BASE, OUT, SIX, ROOT, set_description

text = (ROOT / "v3/descriptions/guidance.md").read_text()
section = text.split("## Corrected descriptions", 1)[1].split("\n## ", 1)[0]
found = dict(re.findall(r"### (rust-[a-z-]+) \(\d+ characters\)\n\n```\n(.*?)\n```", section, re.S))
assert sorted(found) == sorted(SIX), found.keys()

dest = OUT / "v7"
if dest.exists():
    raise SystemExit(f"exists: {dest}")
shutil.copytree(BASE, dest)
for name, desc in found.items():
    assert "\n" not in desc
    set_description(dest / "skills" / name / "SKILL.md", desc)
    print(name, len(desc))
