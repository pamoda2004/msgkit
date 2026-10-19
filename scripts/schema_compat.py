#!/usr/bin/env python3
"""Detects breaking changes between two versions of msgkit's generated JSON Schemas.

The schemas in ``schema/`` are the language-neutral contract: every binding's types derive from
them. Within one major ``SCHEMA_VERSION`` they may only change additively.

    python scripts/schema_compat.py --base-ref origin/main        # compare the working tree to a git ref
    python scripts/schema_compat.py OLD.json NEW.json             # compare two files

Rules (``output`` schemas describe data msgkit produces, ``input`` schemas describe options callers send):

* removing a definition, a property or an enum value is **breaking**;
* changing a property's type is **breaking**;
* output: a property that was required becoming optional is **breaking**;
* input: a new required property, or an optional property becoming required, is **breaking**;
* adding an optional property, a definition, an enum value or a ``oneOf`` variant is **additive**.

Version rules (docs/SCHEMA_POLICY.md), checked against ``SCHEMA_VERSION`` in ``crates/msgkit-core/src/lib.rs``:

* breaking changes are accepted only with a higher major version than the base;
* additive changes need at least a higher minor version than the base;
* the version never goes backwards;
* each schema file's ``x-msgkit-schema-version`` marker equals ``SCHEMA_VERSION``.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any

INPUT_SCHEMAS = {"options.schema.json"}
VERSION_RE = re.compile(r'SCHEMA_VERSION: &str = "(\d+)\.(\d+)\.(\d+)"')

Json = dict[str, Any]
Finding = tuple[str, str]  # (severity "breaking" | "additive", message)


def describe(node: Json | bool) -> str:
    """A stable text description of a (sub)schema's type, without expanding references."""
    if isinstance(node, bool):  # boolean schemas: `true` accepts anything (e.g. arbitrary JSON)
        return "any" if node else "never"
    if "$ref" in node:
        return "ref:" + str(node["$ref"]).rsplit("/", 1)[-1]
    if "const" in node:
        return f"const:{node['const']!r}"
    for key in ("oneOf", "anyOf"):
        if key in node:
            return "union[" + "|".join(sorted(describe(b) for b in node[key])) + "]"
    if "enum" in node:
        return "enum"
    kind = node.get("type")
    if isinstance(kind, list):
        kind = "|".join(sorted(str(k) for k in kind))
    if kind == "array":
        return f"array<{describe(node.get('items', {}))}>"
    return str(kind) if kind is not None else "any"


def shape(node: Json) -> Json:
    """Reduces one definition to the parts that matter for compatibility."""
    props = {name: describe(sub) for name, sub in node.get("properties", {}).items()}
    variants = set()
    enum_values = {json.dumps(v) for v in node.get("enum", [])}
    for key in ("oneOf", "anyOf"):
        for branch in node.get(key, []):
            if isinstance(branch, dict) and "const" in branch and "properties" not in branch:
                enum_values.add(json.dumps(branch["const"]))  # documented enums are emitted as a oneOf of consts
                continue
            tag = branch.get("properties", {}).get("type", {}).get("const")
            variants.add(f"{tag}:{describe(branch)}" if tag is not None else describe(branch))
    return {
        "props": props,
        "required": set(node.get("required", [])),
        "enum": enum_values,
        "variants": variants,
        "kind": describe({k: v for k, v in node.items() if k == "type"}),
    }


def definitions(schema: Json, prefix: str = "") -> dict[str, Json]:
    """Every named definition of a schema file, keyed by name (plus ``<root>``)."""
    result = {f"{prefix}<root>": shape(schema)}
    for name, node in schema.get("$defs", {}).items():
        result[f"{prefix}{name}"] = shape(node)
    return result


def collect(doc: Json) -> dict[str, Json]:
    """Handles both a single schema and a file that bundles several schemas under named keys."""
    looks_single = any(k in doc for k in ("$schema", "$defs", "properties", "oneOf", "type"))
    if looks_single:
        return definitions(doc)
    result: dict[str, Json] = {}
    for key, sub in doc.items():
        if isinstance(sub, dict):  # skips metadata such as the "x-msgkit-schema-version" marker
            result.update(definitions(sub, prefix=f"{key}/"))
    return result


def compare(old_doc: Json, new_doc: Json, direction: str) -> list[Finding]:
    old, new = collect(old_doc), collect(new_doc)
    findings: list[Finding] = []
    for name, o in old.items():
        n = new.get(name)
        if n is None:
            findings.append(("breaking", f"definition removed: {name}"))
            continue
        for prop, desc in o["props"].items():
            if prop not in n["props"]:
                findings.append(("breaking", f"{name}: property removed: {prop}"))
            elif n["props"][prop] != desc:
                findings.append(("breaking", f"{name}.{prop}: type changed {desc} -> {n['props'][prop]}"))
        for prop in n["props"].keys() - o["props"].keys():
            required_now = prop in n["required"]
            if direction == "input" and required_now:
                findings.append(("breaking", f"{name}: new required input property: {prop}"))
            else:
                findings.append(("additive", f"{name}: property added: {prop}"))
        if direction == "output":
            for prop in (o["required"] & set(o["props"])) - n["required"]:
                if prop in n["props"]:
                    findings.append(("breaking", f"{name}.{prop}: required property became optional"))
        else:
            for prop in (n["required"] - o["required"]) & set(o["props"]):
                findings.append(("breaking", f"{name}.{prop}: optional input property became required"))
        for value in o["enum"] - n["enum"]:
            findings.append(("breaking", f"{name}: enum value removed: {value}"))
        for value in n["enum"] - o["enum"]:
            findings.append(("additive", f"{name}: enum value added: {value}"))
        for variant in o["variants"] - n["variants"]:
            findings.append(("breaking", f"{name}: variant removed: {variant}"))
        for variant in n["variants"] - o["variants"]:
            findings.append(("additive", f"{name}: variant added: {variant}"))
        if o["kind"] != n["kind"] and o["kind"] != "any":
            findings.append(("breaking", f"{name}: kind changed {o['kind']} -> {n['kind']}"))
    for name in new.keys() - old.keys():
        findings.append(("additive", f"definition added: {name}"))
    return sorted(findings)


VERSION_KEY = "x-msgkit-schema-version"
Version = tuple[int, int, int]


def marker_version(doc: Json) -> str | None:
    """The version a schema file declares about itself, if any."""
    value = doc.get(VERSION_KEY)
    return value if isinstance(value, str) else None


def version_problems(old_v: Version | None, new_v: Version | None, breaking: bool, additive: bool) -> list[str]:
    """Rules relating schema changes to ``SCHEMA_VERSION`` (docs/SCHEMA_POLICY.md)."""
    if old_v is None or new_v is None:
        return []
    problems = []
    if new_v < old_v:
        problems.append(f"SCHEMA_VERSION went backwards: {old_v} -> {new_v}.")
    if breaking and new_v[0] <= old_v[0]:
        problems.append("Breaking schema changes require a major SCHEMA_VERSION bump.")
    if additive and not breaking and new_v[:2] <= old_v[:2]:
        problems.append("Additive schema changes require at least a minor SCHEMA_VERSION bump.")
    return problems


def parse_version(text: str) -> Version | None:
    m = VERSION_RE.search(text)
    return (int(m.group(1)), int(m.group(2)), int(m.group(3))) if m else None


def git_show(root: Path, ref: str, path: str) -> str | None:
    proc = subprocess.run(["git", "show", f"{ref}:{path}"], cwd=root, capture_output=True, text=True, check=False)
    return proc.stdout if proc.returncode == 0 else None


def check_against_ref(root: Path, ref: str, schema_dir: str) -> int:
    findings: list[tuple[str, Finding]] = []
    for path in sorted((root / schema_dir).glob("*.schema.json")):
        rel = f"{schema_dir}/{path.name}"
        old_text = git_show(root, ref, rel)
        if old_text is None:
            print(f"{rel}: new file (additive)")
            continue
        direction = "input" if path.name in INPUT_SCHEMAS else "output"
        found = compare(json.loads(old_text), json.loads(path.read_text(encoding="utf-8")), direction)
        findings.extend((path.name, f) for f in found)

    breaking = [(f, m) for f, (sev, m) in findings if sev == "breaking"]
    additive = [(f, m) for f, (sev, m) in findings if sev == "additive"]
    for f, m in additive:
        print(f"additive  {f}: {m}")
    for f, m in breaking:
        print(f"BREAKING  {f}: {m}")

    lib = "crates/msgkit-core/src/lib.rs"
    old_v = parse_version(git_show(root, ref, lib) or "")
    new_v = parse_version((root / lib).read_text(encoding="utf-8"))
    problems = version_problems(old_v, new_v, bool(breaking), bool(additive))
    if new_v is not None:
        declared = ".".join(map(str, new_v))
        for path in sorted((root / schema_dir).glob("*.schema.json")):
            marker = marker_version(json.loads(path.read_text(encoding="utf-8")))
            if marker is not None and marker != declared:
                problems.append(f"{path.name} declares schema version {marker} but SCHEMA_VERSION is {declared}.")
    for problem in problems:
        print(f"{problem} (see docs/SCHEMA_POLICY.md)")
    if problems:
        return 1
    if breaking:
        print(f"Breaking changes accepted: SCHEMA_VERSION {old_v} -> {new_v}.")
    else:
        print(f"Schema compatibility OK against {ref} ({len(additive)} additive change(s)).")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("files", nargs="*", help="OLD.json NEW.json")
    parser.add_argument("--base-ref", help="git ref to compare the schema/ directory against")
    parser.add_argument("--schema-dir", default="schema")
    parser.add_argument("--direction", choices=["input", "output"], default="output")
    parser.add_argument("--root", default=".")
    args = parser.parse_args(argv)

    if args.base_ref:
        return check_against_ref(Path(args.root).resolve(), args.base_ref, args.schema_dir)
    if len(args.files) != 2:
        parser.error("give OLD.json NEW.json, or --base-ref REF")
    old = json.loads(Path(args.files[0]).read_text(encoding="utf-8"))
    new = json.loads(Path(args.files[1]).read_text(encoding="utf-8"))
    findings = compare(old, new, args.direction)
    for severity, message in findings:
        print(f"{severity.upper():9} {message}")
    return 1 if any(sev == "breaking" for sev, _ in findings) else 0


if __name__ == "__main__":
    sys.exit(main())
