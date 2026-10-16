"""Builds libmsgkit_ffi.so for every Android ABI into msgkit-android/src/main/jniLibs.

    python packages/kotlin/scripts/build_android_libs.py [--api 26] [--abis arm64-v8a,x86_64]

Requirements:
  * Android NDK (ANDROID_NDK_HOME, or <ANDROID_HOME|ANDROID_SDK_ROOT>/ndk/<version>)
  * Rust targets:  rustup target add aarch64-linux-android armv7-linux-androideabi \
                                     x86_64-linux-android i686-linux-android

No cargo-ndk needed: the NDK's clang wrappers are used directly as the linker.
"""

from __future__ import annotations

import argparse
import os
import platform
import shutil
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
KOTLIN = HERE.parent
REPO = KOTLIN.parent.parent
JNI_LIBS = KOTLIN / "msgkit-android" / "src" / "main" / "jniLibs"

# Rust triple -> (Android ABI directory, clang wrapper prefix)
TARGETS = {
    "aarch64-linux-android": ("arm64-v8a", "aarch64-linux-android"),
    "armv7-linux-androideabi": ("armeabi-v7a", "armv7a-linux-androideabi"),
    "x86_64-linux-android": ("x86_64", "x86_64-linux-android"),
    "i686-linux-android": ("x86", "i686-linux-android"),
}
# Android 15+ devices may use 16 KiB memory pages; Google Play requires 16 KiB
# alignment of native libraries for new apps targeting them.
PAGE_SIZE_FLAGS = "-C link-arg=-Wl,-z,max-page-size=16384"


def find_ndk() -> Path:
    explicit = os.environ.get("ANDROID_NDK_HOME") or os.environ.get("ANDROID_NDK_ROOT")
    if explicit and Path(explicit).is_dir():
        return Path(explicit)
    sdk = os.environ.get("ANDROID_HOME") or os.environ.get("ANDROID_SDK_ROOT")
    candidates = [Path(sdk)] if sdk else []
    if platform.system() == "Windows" and os.environ.get("LOCALAPPDATA"):
        candidates.append(Path(os.environ["LOCALAPPDATA"]) / "Android" / "Sdk")
    for root in candidates:
        versions = sorted((p for p in (root / "ndk").glob("*") if p.is_dir()), key=lambda p: [int(x) if x.isdigit() else 0 for x in p.name.split(".")])
        if versions:
            return versions[-1]
    sys.exit("Android NDK not found. Set ANDROID_NDK_HOME or install the NDK via the SDK manager.")


def host_tag() -> tuple[str, str]:
    system = platform.system()
    if system == "Windows":
        return "windows-x86_64", ".cmd"
    if system == "Darwin":
        return "darwin-x86_64", ""
    return "linux-x86_64", ""


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--api", type=int, default=26, help="Android API level for the linker wrappers (default: 26)")
    parser.add_argument("--abis", help="comma-separated ABI list (default: all four)")
    args = parser.parse_args()

    wanted = set(args.abis.split(",")) if args.abis else None
    ndk = find_ndk()
    tag, suffix = host_tag()
    bin_dir = ndk / "toolchains" / "llvm" / "prebuilt" / tag / "bin"
    print(f"NDK: {ndk}")

    for triple, (abi, prefix) in TARGETS.items():
        if wanted and abi not in wanted:
            continue
        linker = bin_dir / f"{prefix}{args.api}-clang{suffix}"
        if not linker.exists():
            sys.exit(f"linker not found: {linker}")

        env = dict(os.environ)
        key = triple.upper().replace("-", "_")
        env[f"CARGO_TARGET_{key}_LINKER"] = str(linker)
        flags = [env.get(f"CARGO_TARGET_{key}_RUSTFLAGS", "")]
        if triple in ("aarch64-linux-android", "x86_64-linux-android"):
            flags.append(PAGE_SIZE_FLAGS)
        env[f"CARGO_TARGET_{key}_RUSTFLAGS"] = " ".join(f for f in flags if f)
        # A global RUSTFLAGS would take precedence over the per-target variable.
        env.pop("RUSTFLAGS", None)

        subprocess.run(
            ["cargo", "build", "-p", "msgkit-ffi", "--release", "--locked", "--target", triple],
            cwd=REPO, env=env, check=True,
        )
        out = JNI_LIBS / abi
        out.mkdir(parents=True, exist_ok=True)
        shutil.copy2(REPO / "target" / triple / "release" / "libmsgkit_ffi.so", out / "libmsgkit_ffi.so")
        print(f"-> {out / 'libmsgkit_ffi.so'}")


if __name__ == "__main__":
    main()
