"""Builds the native library and a platform wheel for the current machine.

    python packages/python/scripts/build_wheel.py [--out DIST_DIR] [--target TRIPLE]

Needs a Rust toolchain and the ``build`` package (``pip install build``).
The wheel is tagged ``py3-none-<platform>``; set MSGKIT_WHEEL_PLAT to override
the platform tag (e.g. ``manylinux_2_28_x86_64`` on Linux release builds).
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
PKG = HERE.parent
REPO = PKG.parent.parent


def library_name(target: str | None) -> str:
    triple = target or ""
    if "windows" in triple or (not triple and sys.platform == "win32"):
        return "msgkit_ffi.dll"
    if "apple" in triple or (not triple and sys.platform == "darwin"):
        return "libmsgkit_ffi.dylib"
    return "libmsgkit_ffi.so"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", default=str(PKG / "dist"))
    parser.add_argument("--target", help="Rust target triple (default: host)")
    args = parser.parse_args()

    cmd = ["cargo", "build", "-p", "msgkit-ffi", "--release", "--locked"]
    if args.target:
        cmd += ["--target", args.target]
    subprocess.run(cmd, cwd=REPO, check=True)

    name = library_name(args.target)
    built = REPO / "target" / (args.target or "") / "release" / name
    lib_dir = PKG / "src" / "msgkit" / "_lib"
    shutil.rmtree(lib_dir, ignore_errors=True)
    lib_dir.mkdir(parents=True)
    shutil.copy2(built, lib_dir / name)
    shutil.copy2(REPO / "LICENSE", PKG / "LICENSE")

    # Stale build output would leak old files into the wheel.
    shutil.rmtree(PKG / "build", ignore_errors=True)
    subprocess.run(
        [sys.executable, "-m", "build", "--wheel", "--outdir", args.out, str(PKG)],
        check=True,
    )
    print("built:", *sorted(Path(args.out).glob("msgkit_core-*.whl")))


if __name__ == "__main__":
    main()
