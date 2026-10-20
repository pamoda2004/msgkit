"""scripts/sbom.py: path sanitising, reproducible times, and the validation that gates the release.

The validation tests need the CycloneDX library (requirements-sbom.txt) and are skipped without it; the SBOM job in
CI installs it, so they run there.
"""

import json
import os
import sys
import tempfile
import unittest
from collections.abc import Callable
from pathlib import Path
from typing import Any
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import sbom

try:
    import cyclonedx.validation.json  # noqa: F401

    HAVE_LIBRARY = True
except ImportError:
    HAVE_LIBRARY = False


def document(version: str, name: str = "msgkit", components: int = 6) -> dict[str, Any]:
    return {
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "serialNumber": "urn:uuid:3e671687-395b-41f5-a30f-a58921a69b79",
        "version": 1,
        "metadata": {
            "component": {"type": "library", "bom-ref": f"{name}@{version}", "name": name, "version": version}
        },
        "components": [
            {"type": "library", "bom-ref": f"dep{i}@1.0.{i}", "name": f"dep{i}", "version": f"1.0.{i}"}
            for i in range(components)
        ],
    }


def write_set(directory: Path, version: str) -> None:
    for part in sbom.ALL_PARTS:
        text = json.dumps(document(version, name=f"msgkit-{part}"))
        (directory / sbom.filename(part, version)).write_text(text, encoding="utf-8")


class Helpers(unittest.TestCase):
    def test_file_names_carry_the_part_and_the_version(self) -> None:
        self.assertEqual(sbom.filename("ffi", "0.2.0"), "msgkit-ffi-0.2.0.cdx.json")

    def test_the_workspace_version_comes_from_cargo_toml(self) -> None:
        self.assertRegex(sbom.workspace_version(), r"^\d+\.\d+\.\d+")

    def test_checkout_paths_are_removed_from_cargo_package_ids(self) -> None:
        for root in (
            "C:/Users/someone/projects/msgkit",
            "/home/runner/work/msgkit/msgkit",
            "C:/Users/someone/OneDrive/Documents/new%20company/message",
        ):
            text = f'{{"bom-ref": "path+file://{root}/crates/msgkit-ffi#0.2.0", "ref": "path+file://{root}/crates/msgkit-core#0.2.0"}}'
            clean = sbom.sanitize(text)
            self.assertEqual(
                json.loads(clean),
                {
                    "bom-ref": "path+file:///msgkit/crates/msgkit-ffi#0.2.0",
                    "ref": "path+file:///msgkit/crates/msgkit-core#0.2.0",
                },
            )

    def test_the_time_is_the_commit_time_when_source_date_epoch_is_set(self) -> None:
        with mock.patch.dict(os.environ, {"SOURCE_DATE_EPOCH": "1700000000"}):
            self.assertEqual(sbom.timestamp(), "2023-11-14T22:13:20Z")
        with mock.patch.dict(os.environ, {}, clear=False):
            os.environ.pop("SOURCE_DATE_EPOCH", None)
            self.assertRegex(sbom.timestamp(), r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$")


@unittest.skipUnless(HAVE_LIBRARY, "needs the CycloneDX library (requirements-sbom.txt)")
class Validation(unittest.TestCase):
    def check(self, mutate: Callable[[Path], object]) -> list[str]:
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            write_set(directory, "0.2.0")
            mutate(directory)
            return sbom.validate(directory, "0.2.0")

    def test_a_complete_valid_set_passes(self) -> None:
        self.assertEqual(self.check(lambda d: None), [])

    def test_a_missing_file_is_reported(self) -> None:
        problems = self.check(lambda d: (d / sbom.filename("cli", "0.2.0")).unlink())
        self.assertEqual(problems, ["msgkit-cli-0.2.0.cdx.json: missing"])

    def test_a_file_that_is_not_valid_cyclonedx_is_reported(self) -> None:
        def damage(d: Path) -> None:
            path = d / sbom.filename("ffi", "0.2.0")
            broken = json.loads(path.read_text(encoding="utf-8"))
            del broken["bomFormat"]
            path.write_text(json.dumps(broken), encoding="utf-8")

        problems = self.check(damage)
        self.assertEqual(len(problems), 1)
        self.assertIn("msgkit-ffi-0.2.0.cdx.json: not valid CycloneDX", problems[0])

    def test_the_wrong_version_or_the_npm_directory_name_is_reported(self) -> None:
        def wrong_version(d: Path) -> None:
            path = d / sbom.filename("wasm", "0.2.0")
            doc = json.loads(path.read_text(encoding="utf-8"))
            doc["metadata"]["component"]["version"] = "0.1.0"
            path.write_text(json.dumps(doc), encoding="utf-8")

        def directory_name(d: Path) -> None:
            path = d / sbom.filename("node", "0.2.0")
            doc = json.loads(path.read_text(encoding="utf-8"))
            doc["metadata"]["component"]["name"] = "node"
            path.write_text(json.dumps(doc), encoding="utf-8")

        self.assertIn("msgkit-wasm-0.2.0.cdx.json: the root component", self.check(wrong_version)[0])
        self.assertIn("msgkit-node-0.2.0.cdx.json: the root component", self.check(directory_name)[0])

    def test_a_personal_path_or_a_local_file_path_is_reported(self) -> None:
        def leak(d: Path) -> None:
            path = d / sbom.filename("service", "0.2.0")
            doc = json.loads(path.read_text(encoding="utf-8"))
            doc["components"][0]["bom-ref"] = "path+file:///C:/Users/someone/OneDrive/x/crates/y#1"
            path.write_text(json.dumps(doc), encoding="utf-8")

        problems = self.check(leak)
        self.assertTrue(any("personal home path" in p for p in problems), problems)
        self.assertTrue(any("local file path" in p for p in problems), problems)

    def test_a_rust_sbom_with_almost_no_components_is_reported(self) -> None:
        def thin(d: Path) -> None:
            (d / sbom.filename("ffi", "0.2.0")).write_text(json.dumps(document("0.2.0", "msgkit-ffi", components=1)))

        self.assertIn("suspiciously few components", self.check(thin)[0])


if __name__ == "__main__":
    unittest.main()
