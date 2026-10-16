import copy
import json
import sys
import unittest
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import schema_compat as sc

ROOT = Path(__file__).resolve().parents[2]


def load(name: str) -> dict[str, Any]:
    data: dict[str, Any] = json.loads((ROOT / "schema" / name).read_text(encoding="utf-8"))
    return data


def remove_enum_value(definition: dict[str, Any], value: str) -> None:
    """Removes `value` from an enum expressed either as `enum` or as a `oneOf` of consts."""
    if "enum" in definition:
        definition["enum"].remove(value)
    else:
        definition["oneOf"] = [b for b in definition["oneOf"] if b.get("const") != value]


def add_enum_value(definition: dict[str, Any], value: str) -> None:
    if "enum" in definition:
        definition["enum"].append(value)
    else:
        definition["oneOf"].append({"const": value, "type": "string"})


def severities(findings: list[tuple[str, str]]) -> set[str]:
    return {s for s, _ in findings}


class RealSchemas(unittest.TestCase):
    def test_identical_schemas_have_no_findings(self) -> None:
        for name in ("event.schema.json", "import-report.schema.json", "options.schema.json"):
            with self.subTest(name=name):
                doc = load(name)
                self.assertEqual(sc.compare(doc, copy.deepcopy(doc), "output"), [])

    def test_removing_a_property_is_breaking(self) -> None:
        old = load("event.schema.json")
        new = copy.deepcopy(old)
        del new["$defs"]["Message"]["properties"]["body"]
        findings = sc.compare(old, new, "output")
        self.assertIn(("breaking", "Message: property removed: body"), findings)

    def test_changing_a_property_type_is_breaking(self) -> None:
        old = load("event.schema.json")
        new = copy.deepcopy(old)
        new["$defs"]["Call"]["properties"]["duration_secs"]["type"] = "string"
        self.assertEqual(severities(sc.compare(old, new, "output")), {"breaking"})

    def test_removing_an_enum_value_is_breaking_but_adding_one_is_additive(self) -> None:
        old = load("event.schema.json")
        removed = copy.deepcopy(old)
        remove_enum_value(removed["$defs"]["Channel"], "sms")
        self.assertIn("breaking", severities(sc.compare(old, removed, "output")))
        added = copy.deepcopy(old)
        add_enum_value(added["$defs"]["Channel"], "telegram")
        self.assertEqual(severities(sc.compare(old, added, "output")), {"additive"})

    def test_adding_an_optional_property_is_additive(self) -> None:
        old = load("event.schema.json")
        new = copy.deepcopy(old)
        new["$defs"]["Message"]["properties"]["edited_at"] = {"type": ["string", "null"]}
        findings = sc.compare(old, new, "output")
        self.assertEqual(findings, [("additive", "Message: property added: edited_at")])

    def test_required_becoming_optional_breaks_outputs(self) -> None:
        old = load("event.schema.json")
        new = copy.deepcopy(old)
        new["$defs"]["Message"]["required"].remove("sender")
        self.assertIn("breaking", severities(sc.compare(old, new, "output")))

    def test_new_required_input_property_breaks_inputs_only(self) -> None:
        old = load("options.schema.json")
        new = copy.deepcopy(old)
        target = new["whatsapp_export"]
        target["properties"]["new_flag"] = {"type": "boolean"}
        target["required"] = [*target.get("required", []), "new_flag"]
        self.assertIn("breaking", severities(sc.compare(old, new, "input")))
        self.assertEqual(severities(sc.compare(old, new, "output")), {"additive"})

    def test_removing_a_definition_or_variant_is_breaking(self) -> None:
        old = load("event.schema.json")
        no_def = copy.deepcopy(old)
        del no_def["$defs"]["Participant"]
        self.assertIn(("breaking", "definition removed: Participant"), sc.compare(old, no_def, "output"))
        no_variant = copy.deepcopy(old)
        no_variant["oneOf"] = no_variant["oneOf"][:1]
        self.assertIn("breaking", severities(sc.compare(old, no_variant, "output")))


class VersionParsing(unittest.TestCase):
    def test_reads_the_schema_version_constant(self) -> None:
        self.assertEqual(sc.parse_version('pub const SCHEMA_VERSION: &str = "2.14.3";'), (2, 14, 3))
        self.assertIsNone(sc.parse_version("nothing here"))

    def test_the_repository_declares_a_version(self) -> None:
        text = (ROOT / "crates" / "msgkit-core" / "src" / "lib.rs").read_text(encoding="utf-8")
        self.assertIsNotNone(sc.parse_version(text))


if __name__ == "__main__":
    unittest.main()
