"""Builds the wheel, installs it into a clean virtual environment and runs the tests against it.

    python packages/python/scripts/test_wheel.py

This proves the *bundled* native library works (not the development checkout):
the tests run from a temporary directory, outside the repository, so the only
copy of the library they can find is the one inside the installed wheel.
Needs a Rust toolchain and the ``build`` package.
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
PKG = HERE.parent


def run(cmd: list[str], **kwargs: Any) -> None:
    print("+", " ".join(cmd), flush=True)
    subprocess.run(cmd, check=True, **kwargs)


def main() -> None:
    with tempfile.TemporaryDirectory(prefix="msgkit-wheel-") as tmp:
        tmp_path = Path(tmp)
        wheelhouse = tmp_path / "wheelhouse"
        run([sys.executable, str(HERE / "build_wheel.py"), "--out", str(wheelhouse)])
        wheels = sorted(wheelhouse.glob("msgkit_core-*.whl"))
        if len(wheels) != 1:
            raise SystemExit(f"expected exactly one wheel, found {wheels}")
        line = f"python wheel: {wheels[0].name} {wheels[0].stat().st_size} bytes"
        print("SIZE", line, flush=True)
        summary = os.environ.get("GITHUB_STEP_SUMMARY")
        if summary:
            with Path(summary).open("a", encoding="utf-8") as f:
                f.write(line + "\n")

        venv = tmp_path / "venv"
        run([sys.executable, "-m", "venv", str(venv)])
        python = venv / ("Scripts/python.exe" if sys.platform == "win32" else "bin/python")
        run([str(python), "-m", "pip", "install", "--quiet", str(wheels[0])])

        # Show where the library comes from; it must be inside the venv.
        code = "import msgkit._native as n; print([str(p) for p in n._candidates() if p.is_file()][0])"
        out = subprocess.run(
            [str(python), "-c", code], check=True, capture_output=True, text=True, cwd=tmp
        ).stdout.strip()
        print("native library:", out)
        # Compare resolved paths: Windows runners report the temp directory in both 8.3 and long form.
        if not Path(out).resolve().is_relative_to(venv.resolve()):
            raise SystemExit("the wheel did not provide the native library")

        run([str(python), "-m", "unittest", "discover", "-s", str(PKG / "tests"), "-v"], cwd=tmp)


if __name__ == "__main__":
    main()
