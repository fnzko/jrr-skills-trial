"""Build the v3 description variants under v3/variants/.

Usage: python3 -I v3/bin/make_variants.py
Each variant is a copy of v2/arms/skills (v6: v2/arms/short) with only the
frontmatter description lines changed, or skills added/removed for v4 and v5.
"""
import pathlib
import shutil

ROOT = pathlib.Path("/Users/facundo/code/wye/jrr-refactor-trial")
BASE = ROOT / "v2/arms/skills"
SHORT = ROOT / "v2/arms/short"
OUT = ROOT / "v3/variants"

SIX = [
    "rust-comments",
    "rust-domain-types",
    "rust-error-discipline",
    "rust-keep-it-simple",
    "rust-ownership",
    "rust-test-quality",
]

V1 = {
    "rust-comments": "Write, change, refactor or review Rust code so its comments, doc comments, commit messages and pull request text state reasons and contracts instead of restating code or narrating the edit. Covers `//` comments, `///` docs, `SAFETY` comments, TODO markers, commits and pull request descriptions, and diffs whose comments narrate, record history or contradict the code.",
    "rust-domain-types": "Write, change, refactor or review Rust code so domain data is modeled with enums, newtypes and std types instead of bare strings, integers and bools. Covers structs, enums and function signatures, parsing requests, config or other external input, and ids, names, states or units passed around as `String`, `u64` or `bool`.",
    "rust-error-discipline": "Write, change, refactor or review Rust code so failures are handled without hiding them: no silent defaults or discarded `Result`s, panics only for bugs, context added once when an error crosses a layer, and typed errors that keep the cause. Covers `Result` and `Option` handling, fallbacks, input parsing, I/O, casts and arithmetic on input, and `unwrap`, `expect`, `unwrap_or_default`, `let _ =` or `catch_unwind`.",
    "rust-keep-it-simple": "Write, change, refactor or review Rust code so it stays simple: no abstraction, state, helper, dependency or compatibility code the change does not need, and branches, loops and iterator chains in their direct form. Covers new features, traits, generics, helpers, fields and dependencies, changed or copied functions, `if`, `match`, loops and iterator chains, unused code, duplicated logic, shims and debugging leftovers.",
    "rust-ownership": "Write, change, refactor or review Rust code so it avoids needless clones, allocations and repeated work: borrow in signatures, move instead of copying, compute once per input and keep lock scopes short. Covers borrow checker errors, function signatures, getters, loops over collections, code that locks shared state, and `.clone()`, `.to_string()`, `.collect()` or a scan inside a loop.",
    "rust-test-quality": "Write, change, refactor or review Rust tests so they fail when the code is wrong, with exact assertions, expected values that do not come from the code under test, and cases that do not depend on timing or on each other. Covers `#[test]` functions, test modules, files under `tests/`, test helpers, regression tests for a fix, and tests that assert only `is_ok()`, non-emptiness or a substring.",
}

V2_SUFFIX = " Use when building new Rust code, adding a feature to existing Rust code, and refactoring, cleaning up or reviewing existing Rust code."

V3 = {
    "rust-comments": "Load before writing or changing Rust code that adds or edits `//` comments, `///` docs, `SAFETY` comments or TODO markers, and before writing commit messages or pull request text. Comments and messages state reasons and contracts instead of restating code or narrating the edit; also covers reviewing a diff for comments that narrate, record history or contradict the code.",
    "rust-domain-types": "Load before writing or changing Rust code that defines structs, enums or function signatures, parses requests, config or other external input, or passes ids, names, states or units around as `String`, `u64` or `bool`. Models domain data with enums, newtypes and std types instead of bare strings, integers and bools.",
    "rust-error-discipline": "Load before writing or changing Rust code that handles `Result` or `Option`, adds a fallback, parses input, does I/O, casts or adds numbers that come from input, or reaches for `unwrap`, `expect`, `unwrap_or_default`, `let _ =` or `catch_unwind`. Handles failures without hiding them: no silent defaults or discarded `Result`s, panics only for bugs, context added once when an error crosses a layer, and typed errors that keep the cause.",
    "rust-keep-it-simple": "Load before writing or changing Rust code that adds a feature, trait, generic, helper, field or dependency, changes or copies an existing function, or writes `if`, `match`, loops or iterator chains. Keeps the code simple: no abstraction, state, helper, dependency or compatibility code the change does not need, control flow in its direct form, and no unused code, duplicated logic, shims or debugging leftovers.",
    "rust-ownership": "Load before writing or changing Rust code that has function signatures, getters, loops over collections or locks on shared state, that fixes a borrow checker error, or that adds `.clone()`, `.to_string()`, `.collect()` or a scan inside a loop. Avoids needless clones, allocations and repeated work by borrowing in signatures, moving instead of copying, computing once per input and keeping lock scopes short.",
    "rust-test-quality": "Load before writing or changing Rust code that has `#[test]` functions, test modules, files under `tests/`, test helpers or regression tests for a fix. Makes tests fail when the code is wrong: exact assertions, expected values that do not come from the code under test, and cases that do not depend on timing or on each other.",
}

TASK_SKILLS = {
    "rust-new-code": ("Rust rules for new code", "Write new Rust code from scratch: a new crate, a new module, a new binary or library, or implementing a spec or design in Rust. Use first, before writing any Rust source when starting a new crate or module. It holds this project's Rust rules for new code."),
    "rust-refactor": ("Rust rules for cleanup and review", "Clean up, refactor, simplify, review or audit existing Rust code. Use first, before changing any Rust source when the task is to make existing code easier to read or change without changing its behavior, or to review or audit it. It holds this project's Rust rules for cleanup and review."),
    "rust-add-feature": ("Rust rules for changing existing code", "Add a feature to, extend or fix a bug in existing Rust code. Use first, before editing any Rust source when the task changes what an existing crate does: a new feature, endpoint or option, an extension of existing behavior, or a bug fix. It holds this project's Rust rules for changes to existing code."),
}


def set_description(skill_md, text):
    lines = skill_md.read_text().split("\n")
    assert lines[0] == "---"
    idx = next(i for i, l in enumerate(lines) if l.startswith("description: "))
    assert idx < lines.index("---", 1)
    lines[idx] = "description: " + text
    skill_md.write_text("\n".join(lines))


def description_of(skill_md):
    for line in skill_md.read_text().split("\n"):
        if line.startswith("description: "):
            return line[len("description: "):]
    raise ValueError(skill_md)


def copy_base(name, src=BASE):
    dest = OUT / name
    if dest.exists():
        raise SystemExit(f"exists: {dest}")
    shutil.copytree(src, dest)
    return dest


def add_task_skills(dest):
    body = (SHORT / "skills/rust-rules/SKILL.md").read_text().split("---\n", 2)[2]
    body = body.replace("# Rust rules for this project\n", "", 1)
    for name, (title, desc) in TASK_SKILLS.items():
        d = dest / "skills" / name
        d.mkdir()
        (d / "SKILL.md").write_text(f"---\nname: {name}\ndescription: {desc}\n---\n\n# {title}\n{body}")


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    copy_base("v0")
    d = copy_base("v1")
    for s, t in V1.items():
        set_description(d / "skills" / s / "SKILL.md", t)
    d = copy_base("v2")
    for s in SIX:
        f = d / "skills" / s / "SKILL.md"
        set_description(f, description_of(f) + V2_SUFFIX)
    d = copy_base("v3")
    for s, t in V3.items():
        set_description(d / "skills" / s / "SKILL.md", t)
    d = copy_base("v4")
    for s in SIX:
        shutil.rmtree(d / "skills" / s)
    add_task_skills(d)
    d = copy_base("v5")
    add_task_skills(d)
    copy_base("v6", SHORT)


if __name__ == "__main__":
    main()
