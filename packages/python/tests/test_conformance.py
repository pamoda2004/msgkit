"""Runs the language-independent conformance suite (``conformance/``) through the Python binding.

The expected files hold the canonical response envelope; every binding must reproduce them byte for byte.
See ``conformance/README.md`` for the case format and the canonical JSON rules.
"""

import json
import unittest
from pathlib import Path
from typing import Any

from msgkit import MsgkitError
from msgkit._native import dispatch

CASES = Path(__file__).resolve().parents[3] / "conformance" / "cases"

_SHORT_ESCAPES = {'"': '\\"', "\\": "\\\\", "\b": "\\b", "\f": "\\f", "\n": "\\n", "\r": "\\r", "\t": "\\t"}


def _string(value: str) -> str:
    out = ['"']
    for char in value:
        if char in _SHORT_ESCAPES:
            out.append(_SHORT_ESCAPES[char])
        elif ord(char) < 0x20:
            out.append(f"\\u{ord(char):04x}")
        else:
            out.append(char)
    out.append('"')
    return "".join(out)


def canonical(value: Any) -> str:
    """Compact JSON with keys sorted by code point and minimal string escapes (integers only)."""
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, int):
        return str(value)
    if isinstance(value, float):
        raise AssertionError(f"conformance output must not contain floats: {value}")
    if isinstance(value, str):
        return _string(value)
    if isinstance(value, list):
        return "[" + ",".join(canonical(item) for item in value) + "]"
    if isinstance(value, dict):
        return "{" + ",".join(f"{_string(k)}:{canonical(value[k])}" for k in sorted(value)) + "}"
    raise AssertionError(f"unexpected value {value!r}")


def load_input(directory: Path, spec: dict[str, Any]) -> dict[str, Any]:
    payload: dict[str, Any] = dict(spec["input"])
    for key, name in spec.get("files", {}).items():
        payload[key] = (directory / name).read_text(encoding="utf-8")
    for key, name in spec.get("json_files", {}).items():
        payload[key] = json.loads((directory / name).read_text(encoding="utf-8"))
    return payload


def envelope(op: str, payload: dict[str, Any]) -> dict[str, Any]:
    try:
        return {"ok": True, "data": dispatch(op, payload)}
    except MsgkitError as error:
        return {"ok": False, "error": {"code": error.code, "message": error.message}}


class ConformanceTests(unittest.TestCase):
    def test_every_case_is_reproduced_byte_for_byte(self) -> None:
        directories = sorted(p for p in CASES.iterdir() if p.is_dir())
        self.assertGreaterEqual(len(directories), 40)
        for directory in directories:
            with self.subTest(case=directory.name):
                spec = json.loads((directory / "case.json").read_text(encoding="utf-8"))
                actual = canonical(envelope(spec["op"], load_input(directory, spec))) + "\n"
                expected = (directory / "expected.json").read_bytes()
                self.assertEqual(actual.encode("utf-8"), expected)


if __name__ == "__main__":
    unittest.main()
