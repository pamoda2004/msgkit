#!/usr/bin/env python3
"""Generates the TypeScript, Python and Kotlin data types from the JSON Schemas in ``schema/``.

    python scripts/generate_types.py            # rewrite the generated regions
    python scripts/generate_types.py --check    # exit 1 if a generated region differs from the schema

The generated code lives between the markers

    <generated types: do not edit ...>  ...  </generated types>

in ``packages/node/src/index.d.ts``, ``packages/python/src/msgkit/types.py`` and
``packages/kotlin/.../Models.kt``. Everything outside the markers (functions, error classes, helpers and the
few types that are not part of the schema, such as ``VersionInfo``) is hand-written.

Only the Python standard library is used, so the output is identical on every platform.

Two assumptions about msgkit's *output* (``event`` and ``import-report`` schemas), both enforced by
``scripts/tests/test_generate_types.py`` against the conformance suite:

* msgkit never writes ``null``: an absent value is omitted. So nullable output properties are generated as
  plain optional properties.
* A property with a ``default`` (``recipients``, ``attachments``, ``participants``, ``video``) is always
  written, so it is generated as required even though the schema lists it as optional for readers.

*Input* schemas (``options``) are generated exactly as the schema says.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import textwrap
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
SCHEMA_DIR = ROOT / "schema"
TS_FILE = ROOT / "packages/node/src/index.d.ts"
PY_FILE = ROOT / "packages/python/src/msgkit/types.py"
KT_FILE = ROOT / "packages/kotlin/msgkit/src/main/kotlin/io/github/pamoda2004/msgkit/Models.kt"

BEGIN = "<generated types: do not edit; run `python scripts/generate_types.py` after changing schema/>"
END = "</generated types>"

Json = dict[str, Any]

# ---------------------------------------------------------------------------------------------------------
# Presentation overlay. The schema does not keep declaration order (JSON Schema objects are sorted), and the
# order is public API (enum ordinals, Kotlin constructor parameters). Items the overlay does not mention are
# appended in schema order, so a new property or enum value needs no change here.
# ---------------------------------------------------------------------------------------------------------

TYPE_ORDER = [
    "Channel",
    "Direction",
    "CallOutcome",
    "AttachmentKind",
    "Participant",
    "Attachment",
    "Message",
    "Call",
    "ImportReport",
    "DateOrder",
    "WhatsAppExportOptions",
    "FacebookExportOptions",
    "RawSms",
    "RawCall",
    "DeviceBatch",
    "DeviceOptions",
]

ENUM_ORDER = {
    "Channel": ["sms", "cellular", "whatsapp", "messenger"],
    "Direction": ["inbound", "outbound", "unknown"],
    "CallOutcome": ["answered", "missed", "rejected", "blocked", "voicemail", "unknown"],
    "AttachmentKind": ["image", "video", "audio", "file", "other"],
    "DateOrder": ["auto", "dmy", "mdy"],
}

_EVENT_COMMON = ["id", "channel", "external_id", "conversation_id", "direction", "timestamp"]
PROP_ORDER = {
    "ts": {
        "Participant": ["id", "display_name"],
        "Attachment": ["kind", "uri", "mime_type", "file_name", "source_id"],
        "Message": [*_EVENT_COMMON, "sender", "recipients", "body", "attachments", "raw"],
        "Call": [*_EVENT_COMMON, "duration_secs", "outcome", "video", "participants", "raw"],
        "WhatsAppExportOptions": [
            "self_name",
            "date_order",
            "utc_offset_minutes",
            "conversation_id",
            "default_country_code",
        ],
        "FacebookExportOptions": ["self_name", "conversation_id"],
        "RawSms": ["id", "address", "body", "date_ms", "type", "thread_id"],
        "RawCall": ["id", "number", "name", "date_ms", "duration_secs", "type"],
        "DeviceBatch": ["sms", "calls"],
        "DeviceOptions": ["own_number", "default_country_code"],
    },
    "kt": {
        "Participant": ["id", "display_name"],
        "Attachment": ["kind", "uri", "mime_type", "file_name", "source_id"],
        "Message": [*_EVENT_COMMON, "sender", "recipients", "body", "attachments", "raw"],
        "Call": [*_EVENT_COMMON, "duration_secs", "outcome", "video", "participants", "raw"],
        "WhatsAppExportOptions": [
            "self_name",
            "date_order",
            "utc_offset_minutes",
            "conversation_id",
            "default_country_code",
        ],
        "FacebookExportOptions": ["self_name", "conversation_id"],
        "RawSms": ["address", "body", "date_ms", "type", "id", "thread_id"],
        "RawCall": ["number", "date_ms", "type", "duration_secs", "name", "id"],
        "DeviceBatch": ["sms", "calls"],
        "DeviceOptions": ["own_number", "default_country_code"],
    },
}

# Kotlin property names that differ from the camel-cased JSON name.
KT_RENAMES = {("RawSms", "type"): "kind", ("RawCall", "type"): "kind"}

# Kotlin keeps decoding when a required output property is missing.
KT_LENIENT_DEFAULTS = {("ImportReport", "warnings"): "emptyList()"}

# Hand-written Kotlin that belongs to a generated type (kept inside the generated region so the type stays whole).
KT_AFTER_TYPE = {
    "Event": (
        "/** [Event.timestamp] as an [Instant] (needs Android API 26+, or core library desugaring). */\n"
        "public val Event.instant: Instant get() = Instant.parse(timestamp)\n"
    ),
}
KT_BODY_EXTRAS = {
    "ImportReport": (
        "    public val messages: List<Message> get() = events.filterIsInstance<Message>()\n"
        "    public val calls: List<Call> get() = events.filterIsInstance<Call>()\n"
    ),
}

OUTPUT_FILES = {"event.schema.json", "import-report.schema.json"}


# ---------------------------------------------------------------------------------------------------------
# Intermediate representation
# ---------------------------------------------------------------------------------------------------------


@dataclass
class Ty:
    kind: str  # string | int | bool | any | ref | array
    arg: Any = None  # int: "int32" | "int64" | "uint64"; ref: type name; array: element Ty


@dataclass
class Prop:
    name: str
    ty: Ty
    doc: str | None
    required: bool  # as generated (after the output profile)
    nullable: bool
    has_default: bool
    default: Any = None


@dataclass
class TypeDef:
    name: str
    kind: str  # object | enum
    doc: str | None
    output: bool
    props: list[Prop] = field(default_factory=list)
    values: list[tuple[str, str | None]] = field(default_factory=list)  # enum: (value, doc)


@dataclass
class Model:
    types: dict[str, TypeDef]
    variants: list[tuple[str, str]]  # event tag -> type name
    union_name: str
    union_doc: str | None


def clean_doc(text: str | None) -> str | None:
    if not text:
        return None
    return re.sub(r"\s+", " ", text).strip() or None


def resolve(node: Json | bool) -> tuple[Ty, bool]:
    """Returns the type of a (sub)schema and whether it admits ``null``."""
    if isinstance(node, bool):
        return Ty("any"), False
    if "$ref" in node:
        return Ty("ref", node["$ref"].rsplit("/", 1)[-1]), False
    kind = node.get("type")
    nullable = False
    if isinstance(kind, list):
        nullable = "null" in kind
        kind = next(k for k in kind if k != "null")
    if kind == "array":
        inner, _ = resolve(node.get("items", True))
        return Ty("array", inner), nullable
    if kind == "string":
        return Ty("string"), nullable
    if kind == "integer":
        return Ty("int", node.get("format", "int64")), nullable
    if kind == "boolean":
        return Ty("bool"), nullable
    return Ty("any"), nullable


def enum_values(node: Json) -> list[tuple[str, str | None]]:
    if "enum" in node:
        return [(v, None) for v in node["enum"]]
    values: list[tuple[str, str | None]] = []
    for branch in node.get("oneOf", []):
        if "const" in branch:
            values.append((branch["const"], clean_doc(branch.get("description"))))
        else:
            values.extend((v, None) for v in branch.get("enum", []))
    return values


def order_enum(name: str, values: list[tuple[str, str | None]]) -> list[tuple[str, str | None]]:
    preferred = ENUM_ORDER.get(name, [])
    rank = {v: i for i, v in enumerate(preferred)}
    return sorted(values, key=lambda item: rank.get(item[0], len(preferred)))


def build_object(name: str, node: Json, output: bool) -> TypeDef:
    required = set(node.get("required", []))
    props = []
    for pname, sub in node.get("properties", {}).items():
        ty, nullable = resolve(sub)
        has_default = isinstance(sub, dict) and "default" in sub
        default = sub.get("default") if isinstance(sub, dict) else None
        is_required = pname in required
        if output:
            # msgkit omits absent values and always writes defaulted ones (see the module docstring).
            nullable = False
            is_required = is_required or has_default
        props.append(
            Prop(
                pname,
                ty,
                clean_doc(sub.get("description")) if isinstance(sub, dict) else None,
                is_required,
                nullable,
                has_default,
                default,
            )
        )
    return TypeDef(name, "object", clean_doc(node.get("description")), output, props)


def build_enum(name: str, node: Json) -> TypeDef:
    return TypeDef(name, "enum", clean_doc(node.get("description")), False, values=order_enum(name, enum_values(node)))


def load_model(schema_dir: Path = SCHEMA_DIR) -> Model:
    event = json.loads((schema_dir / "event.schema.json").read_text(encoding="utf-8"))
    report = json.loads((schema_dir / "import-report.schema.json").read_text(encoding="utf-8"))
    options = json.loads((schema_dir / "options.schema.json").read_text(encoding="utf-8"))
    types: dict[str, TypeDef] = {}

    def add_defs(defs: Json, output: bool) -> None:
        for name, node in defs.items():
            if "properties" in node:
                types[name] = build_object(name, node, output)
            else:
                types[name] = build_enum(name, node)

    add_defs(event.get("$defs", {}), output=True)
    types["ImportReport"] = build_object("ImportReport", report, output=True)
    for sub in options.values():
        if isinstance(sub, dict):  # skips the "x-msgkit-schema-version" marker
            add_defs(sub.get("$defs", {}), output=False)
            types[sub["title"]] = build_object(sub["title"], sub, output=False)

    variants = []
    for branch in event["oneOf"]:
        tag = branch["properties"]["type"]["const"]
        variants.append((tag, branch["$ref"].rsplit("/", 1)[-1]))
    return Model(types, variants, event["title"], clean_doc(event.get("description")))


def ordered_types(model: Model) -> list[TypeDef]:
    rank = {n: i for i, n in enumerate(TYPE_ORDER)}
    names = sorted(model.types, key=lambda n: rank.get(n, len(TYPE_ORDER)))
    return [model.types[n] for n in names]


def ordered_props(td: TypeDef, lang: str) -> list[Prop]:
    preferred = PROP_ORDER[lang].get(td.name, [])
    rank = {n: i for i, n in enumerate(preferred)}
    return sorted(td.props, key=lambda p: rank.get(p.name, len(preferred)))


def camel(name: str) -> str:
    head, *rest = name.split("_")
    return head + "".join(part.capitalize() for part in rest)


def doc_lines(text: str, width: int) -> list[str]:
    return textwrap.wrap(text, width=width, break_long_words=False, break_on_hyphens=False)


# ---------------------------------------------------------------------------------------------------------
# TypeScript
# ---------------------------------------------------------------------------------------------------------


def ts_type(ty: Ty) -> str:
    if ty.kind == "string":
        return "string"
    if ty.kind == "int":
        return "number"
    if ty.kind == "bool":
        return "boolean"
    if ty.kind == "ref":
        return str(ty.arg)
    if ty.kind == "array":
        return ts_type(ty.arg) + "[]"
    return "unknown"


def ts_doc(text: str | None, indent: str = "") -> list[str]:
    if not text:
        return []
    if len(text) + len(indent) + 8 <= 110:
        return [f"{indent}/** {text} */"]
    return [f"{indent}/**", *[f"{indent} * {line}" for line in doc_lines(text, 100 - len(indent))], f"{indent} */"]


def gen_ts(model: Model) -> str:
    out: list[str] = []
    for td in ordered_types(model):
        if td.kind == "enum":
            out.append(f"export type {td.name} = " + " | ".join(f"'{v}'" for v, _ in td.values) + ";")
            continue
        out.extend(ts_doc(td.doc))
        out.append(f"export interface {td.name} {{")
        for p in ordered_props(td, "ts"):
            out.extend(ts_doc(p.doc, "  "))
            mark = "" if p.required else "?"
            null = " | null" if p.nullable else ""
            out.append(f"  {p.name}{mark}: {ts_type(p.ty)}{null};")
        out.append("}")
        out.append("")
        if td.name == "Call":
            for tag, name in model.variants:
                out.append(f"export type {name}Event = {name} & {{ type: '{tag}' }};")
            out.append(f"export type {model.union_name} = " + " | ".join(f"{n}Event" for _, n in model.variants) + ";")
            out.append("")
    # enums are one-liners; drop the blank line that follows the last of them only once
    text = "\n".join(out).rstrip("\n")
    return re.sub(r"(export type \w+ = [^\n]*;)\n(export interface|/\*\*)", r"\1\n\n\2", text) + "\n"


# ---------------------------------------------------------------------------------------------------------
# Python
# ---------------------------------------------------------------------------------------------------------


def py_type(ty: Ty) -> str:
    if ty.kind == "string":
        return "str"
    if ty.kind == "int":
        return "int"
    if ty.kind == "bool":
        return "bool"
    if ty.kind == "ref":
        return str(ty.arg)
    if ty.kind == "array":
        return f"List[{py_type(ty.arg)}]"
    return "Any"


def py_field(p: Prop, tag: str | None = None) -> list[str]:
    ann = f'Literal["{tag}"]' if tag else py_type(p.ty)
    if p.nullable:
        ann = f"Optional[{ann}]"
    lines = [f"    # {line}" for line in doc_lines(p.doc, 100)] if p.doc else []
    return [*lines, f"    {p.name}: {ann}"]


def gen_py(model: Model) -> str:
    out: list[str] = []
    tag_of = {name: tag for tag, name in model.variants}
    for td in ordered_types(model):
        if td.kind == "enum":
            out.append(f"{td.name} = Literal[" + ", ".join(f'"{v}"' for v, _ in td.values) + "]")
    out.append("")
    for td in ordered_types(model):
        if td.kind != "object":
            continue
        props = ordered_props(td, "py") if "py" in PROP_ORDER else ordered_props(td, "ts")
        fields: list[tuple[Prop, str | None]] = []
        if td.name in tag_of:  # the event discriminator is a field of the Python TypedDicts
            fields.append(
                (Prop("type", Ty("string"), None, True, False, False), tag_of[td.name]),
            )
        fields.extend((p, None) for p in props)
        required = [(p, t) for p, t in fields if p.required]
        optional = [(p, t) for p, t in fields if not p.required]
        out.append("")
        if required and optional:
            out.extend(["", f"class _{td.name}Required(TypedDict):"])
            for p, t in required:
                out.extend(py_field(p, t))
            out.extend(["", "", f"class {td.name}(_{td.name}Required, total=False):"])
            for p, t in optional:
                out.extend(py_field(p, t))
        else:
            head = "TypedDict" if required else "TypedDict, total=False"
            out.extend(["", f"class {td.name}({head}):"])
            for p, t in fields:
                out.extend(py_field(p, t))
        if td.name == "Call":
            out.extend(["", "", f"{model.union_name} = Union[" + ", ".join(n for _, n in model.variants) + "]"])
    text = "\n".join(out).strip("\n")
    return re.sub(r"\n{4,}", "\n\n\n", text) + "\n"


# ---------------------------------------------------------------------------------------------------------
# Kotlin
# ---------------------------------------------------------------------------------------------------------


def kt_type(ty: Ty) -> str:
    if ty.kind == "string":
        return "String"
    if ty.kind == "int":
        return "Int" if ty.arg == "int32" else "Long"
    if ty.kind == "bool":
        return "Boolean"
    if ty.kind == "ref":
        return str(ty.arg)
    if ty.kind == "array":
        return f"List<{kt_type(ty.arg)}>"
    return "JsonElement"


def kt_doc(text: str | None, indent: str = "") -> list[str]:
    if not text:
        return []
    if len(text) + len(indent) + 8 <= 110:
        return [f"{indent}/** {text} */"]
    return [f"{indent}/**", *[f"{indent} * {line}" for line in doc_lines(text, 100 - len(indent))], f"{indent} */"]


def kt_default(td: TypeDef, p: Prop, types: dict[str, TypeDef]) -> str | None:
    lenient = KT_LENIENT_DEFAULTS.get((td.name, p.name))
    if lenient:
        return lenient
    if p.has_default:
        value = p.default
        if value is None:
            return "null"
        if p.ty.kind == "array":
            return "emptyList()"
        if p.ty.kind == "bool":
            return "true" if value else "false"
        if p.ty.kind == "int":
            return str(value)
        if p.ty.kind == "ref" and isinstance(value, str):
            return f"{p.ty.arg}.{value.upper()}"
        if p.ty.kind == "ref" and isinstance(value, dict):
            return f"{p.ty.arg}()"  # a struct whose own properties all have defaults
        if value is None:
            return "null"
    if not p.required and (p.nullable or p.ty.kind == "any" or td.output):
        return "null"
    return None


def kt_property(td: TypeDef, p: Prop, types: dict[str, TypeDef], override: bool, last: bool) -> list[str]:
    name = KT_RENAMES.get((td.name, p.name), camel(p.name))
    ann = f'@SerialName("{p.name}") ' if name != p.name else ""
    typ = kt_type(p.ty)
    default = kt_default(td, p, types)
    nullable = (p.nullable or (not p.required and default == "null")) and not typ.startswith("List")
    if default == "null":
        nullable = True
    prefix = "override " if override else ""
    line = f"    {ann}{prefix}val {name}: {typ}{'?' if nullable else ''}"
    if default is not None:
        line += f" = {default}"
    return [*kt_doc(p.doc, "    "), line + ","]


def gen_kt(model: Model) -> str:
    out: list[str] = []
    tag_of = {name: tag for tag, name in model.variants}
    variant_types = [model.types[name] for _, name in model.variants]
    common = [
        p.name
        for p in ordered_props(variant_types[0], "kt")
        if p.required
        and all(any(q.name == p.name and q.required and q.ty == p.ty for q in v.props) for v in variant_types)
    ]
    for td in ordered_types(model):
        if td.kind == "enum":
            out.extend(kt_doc(td.doc))
            out.extend(["@Serializable", f"public enum class {td.name} {{"])
            for i, (value, doc) in enumerate(td.values):
                if i:
                    out.append("")
                out.extend(kt_doc(doc, "    "))
                out.extend([f'    @SerialName("{value}")', f"    {value.upper()},"])
            out.extend(["}", ""])
            continue
        out.extend(kt_doc(td.doc))
        out.append("@Serializable")
        if td.name in tag_of:
            out.append(f'@SerialName("{tag_of[td.name]}")')
        props = ordered_props(td, "kt")
        extras = KT_BODY_EXTRAS.get(td.name)
        suffix = f" : {model.union_name}" if td.name in tag_of else ""
        body = f") {{\n{extras}}}" if extras else ")"
        out.append(f"public data class {td.name}(")
        for i, p in enumerate(props):
            out.extend(kt_property(td, p, model.types, p.name in common and td.name in tag_of, i == len(props) - 1))
        out.append(body + suffix)
        out.append("")
        if td.name == "Call":
            # The sealed interface that Message and Call implement is declared before them, see below.
            pass
    text = "\n".join(out)
    interface = kt_union(model, common, variant_types)
    # Insert the sealed interface (and its hand-written extension) before the first variant.
    first_variant = f"/** {model.types[model.variants[0][1]].doc} */" if model.types[model.variants[0][1]].doc else ""
    marker = first_variant + "\n@Serializable\n@SerialName"
    text = text.replace(marker, interface + marker, 1) if first_variant else interface + text
    return text.rstrip("\n") + "\n"


def kt_union(model: Model, common: list[str], variants: list[TypeDef]) -> str:
    lines = [*kt_doc(model.union_doc), "@Serializable", f"public sealed interface {model.union_name} {{"]
    first = True
    for name in common:
        prop = next(p for p in variants[0].props if p.name == name)
        doc = kt_doc(prop.doc, "    ")
        if doc and not first:
            lines.append("")
        lines.extend(doc)
        lines.append(f"    public val {camel(name)}: {kt_type(prop.ty)}")
        first = False
    lines.extend(["}", ""])
    after = KT_AFTER_TYPE.get(model.union_name)
    if after:
        lines.extend([after])
    return "\n".join(lines) + "\n"


# ---------------------------------------------------------------------------------------------------------
# Files
# ---------------------------------------------------------------------------------------------------------


def comment_style(path: Path) -> str:
    return "#" if path.suffix == ".py" else "//"


def region_pattern(path: Path) -> re.Pattern[str]:
    c = re.escape(comment_style(path))
    return re.compile(rf"({c} {re.escape(BEGIN)}\n)(.*?)({c} {re.escape(END)})", re.S)


def replace_region(path: Path, text: str, generated: str) -> str:
    pattern = region_pattern(path)
    if not pattern.search(text):
        raise SystemExit(f"{path.relative_to(ROOT)}: generated-region markers not found")
    return pattern.sub(lambda m: m.group(1) + lead(path) + generated + gap(path) + m.group(3), text, count=1)


def lead(path: Path) -> str:
    """Kotlin: ktlint wants a blank line between an end-of-line comment and a following KDoc."""
    return "\n" if path.suffix == ".kt" else ""


def gap(path: Path) -> str:
    """Blank lines between the generated code and the closing marker (what the language's formatter expects)."""
    return "\n\n" if path.suffix == ".py" else "\n"


def targets(model: Model) -> list[tuple[Path, str]]:
    return [(TS_FILE, gen_ts(model)), (PY_FILE, gen_py(model)), (KT_FILE, gen_kt(model))]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", action="store_true", help="fail if a generated region is out of date")
    args = parser.parse_args(argv)
    model = load_model()
    stale = []
    for path, generated in targets(model):
        current = path.read_text(encoding="utf-8").replace("\r\n", "\n")
        updated = replace_region(path, current, generated)
        if updated != current:
            stale.append(path)
            if not args.check:
                path.write_text(updated, encoding="utf-8", newline="\n")
    names = ", ".join(str(p.relative_to(ROOT)) for p in stale)
    if args.check and stale:
        print(
            f"Generated types are out of date: {names}\nRun `python scripts/generate_types.py` and commit the result."
        )
        return 1
    print(f"Updated: {names}" if stale else "Generated types are up to date.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
