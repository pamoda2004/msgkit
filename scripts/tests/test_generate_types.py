import copy
import json
import sys
import unittest
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import generate_types as gt

ROOT = Path(__file__).resolve().parents[2]


def nulls_outside_raw(value: Any, path: str = "") -> list[str]:
    if value is None:
        return [path]
    found: list[str] = []
    if isinstance(value, dict):
        for key, sub in value.items():
            if key != "raw":  # `raw` holds the provider's own payload, which may contain anything
                found += nulls_outside_raw(sub, f"{path}/{key}")
    elif isinstance(value, list):
        for i, sub in enumerate(value):
            found += nulls_outside_raw(sub, f"{path}[{i}]")
    return found


class RepositoryIsInSync(unittest.TestCase):
    def test_generated_regions_match_the_schema(self) -> None:
        self.assertEqual(gt.main(["--check"]), 0)

    def test_every_target_has_its_markers(self) -> None:
        for path in (gt.TS_FILE, gt.PY_FILE, gt.KT_FILE):
            text = path.read_text(encoding="utf-8")
            self.assertTrue(gt.region_pattern(path).search(text), path.name)

    def test_generation_is_deterministic(self) -> None:
        first = [generated for _, generated in gt.targets(gt.load_model())]
        second = [generated for _, generated in gt.targets(gt.load_model())]
        self.assertEqual(first, second)

    def test_a_stale_file_is_detected(self) -> None:
        generated = dict(gt.targets(gt.load_model()))[gt.TS_FILE]
        text = gt.TS_FILE.read_text(encoding="utf-8")
        self.assertEqual(gt.replace_region(gt.TS_FILE, text, generated), text)
        self.assertNotEqual(gt.replace_region(gt.TS_FILE, text, generated + "export type Extra = string;\n"), text)


class SchemaChangesReachEveryBinding(unittest.TestCase):
    def test_a_new_property_and_enum_value_are_generated_everywhere(self) -> None:
        model = gt.load_model()
        message = model.types["Message"]
        message.props.append(gt.Prop("zz_new_field", gt.Ty("string"), "A new field.", False, False, False))
        model.types["Channel"].values.append(("signal", None))
        ts, py, kt = gt.gen_ts(model), gt.gen_py(model), gt.gen_kt(model)
        self.assertIn("zz_new_field?: string;", ts)
        self.assertIn("zz_new_field: str", py)
        self.assertIn('@SerialName("zz_new_field") val zzNewField: String? = null', kt)
        self.assertIn("'signal'", ts.split("export type Direction")[0])
        self.assertIn('"signal"', py)
        self.assertIn('@SerialName("signal")', kt)

    def test_unknown_enum_values_come_after_the_known_ones(self) -> None:
        model = gt.load_model()
        values = [v for v, _ in gt.order_enum("Channel", [("signal", None), *model.types["Channel"].values])]
        self.assertEqual(values, ["sms", "cellular", "whatsapp", "messenger", "signal"])

    def test_enum_order_follows_the_declared_order(self) -> None:
        model = gt.load_model()
        for name, order in gt.ENUM_ORDER.items():
            self.assertEqual([v for v, _ in model.types[name].values], order, name)

    def test_removing_a_property_from_the_schema_removes_it_from_the_output(self) -> None:
        model = gt.load_model()
        model.types["Participant"].props = [p for p in model.types["Participant"].props if p.name != "display_name"]
        self.assertNotIn("display_name", gt.gen_ts(copy.deepcopy(model)).split("export interface Participant")[1][:200])


class OutputAssumptionsHold(unittest.TestCase):
    """The generator treats output as 'never null, defaulted properties always present'."""

    def test_conformance_outputs_have_no_nulls_and_always_write_defaulted_properties(self) -> None:
        events = 0
        for expected in sorted((ROOT / "conformance" / "cases").glob("*/expected.json")):
            doc = json.loads(expected.read_text(encoding="utf-8"))
            for event in (doc.get("data") or {}).get("events", []):
                events += 1
                self.assertEqual(nulls_outside_raw(event), [], expected.parent.name)
                always = ["recipients", "attachments"] if event["type"] == "message" else ["video", "participants"]
                for key in always:
                    self.assertIn(key, event, f"{expected.parent.name}: {key}")
        self.assertGreater(events, 50)

    def test_the_assumed_properties_are_exactly_those_with_defaults(self) -> None:
        model = gt.load_model()
        defaulted = sorted(
            f"{td.name}.{p.name}" for td in model.types.values() if td.output for p in td.props if p.has_default
        )
        self.assertEqual(defaulted, ["Call.participants", "Call.video", "Message.attachments", "Message.recipients"])


if __name__ == "__main__":
    unittest.main()
