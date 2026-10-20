"""Generates the CycloneDX software bills of materials (SBOMs) that a release ships, and checks them.

    python scripts/sbom.py --out sbom                 # all of them
    python scripts/sbom.py --out sbom --only rust     # a subset: rust, node, python, kotlin
    python scripts/sbom.py --validate-only sbom       # just check a directory

One SBOM per released artifact, CycloneDX 1.5 JSON, named ``msgkit-<part>-<version>.cdx.json``:

* ``ffi``, ``wasm``, ``service``, ``cli``: the Rust dependency trees (``cargo cyclonedx``). ``ffi`` is the native
  library inside every wheel, JAR and AAR; ``wasm`` is inside the npm package.
* ``node``: the npm package (``npm sbom``), ``python``: the wheel, ``kotlin`` and ``android``: the JAR and the AAR
  (the Gradle CycloneDX plugin).

Local paths never reach an SBOM (Cargo records the checkout path in its package ids; it is replaced), the time is the
time of the commit when ``SOURCE_DATE_EPOCH`` is set (the release workflow sets it), and every file is validated
against the CycloneDX 1.5 JSON schema. Needs ``cargo-cyclonedx``, npm, a JDK and
``cyclonedx-python-lib[validation]`` (pinned in requirements-dev.txt and in the release workflow).
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(Path(__file__).resolve().parent))

import check_repo_hygiene as hygiene  # noqa: E402  (reuses the personal-path rule)

SPEC_VERSION = "1.5"
RUST_PARTS = {  # part name -> (crate directory, file the cargo-cyclonedx run produces)
    "ffi": ("msgkit-ffi", "msgkit_ffi_cdylib-staticlib-rlib.cdx.json"),
    "wasm": ("msgkit-wasm", "msgkit_wasm_cdylib.cdx.json"),
    "service": ("msgkit-service", "msgkit-service_bin.cdx.json"),
    "cli": ("msgkit-cli", "msgkit_bin.cdx.json"),
}
ALL_PARTS = [*RUST_PARTS, "node", "python", "kotlin", "android"]
# The Windows runner's `npm` and `gradlew` are scripts, not programs.
WINDOWS = sys.platform == "win32"


def workspace_version() -> str:
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    match = re.search(r'(?m)^version = "([^"]+)"', text)
    if not match:
        raise SystemExit("no workspace version in Cargo.toml")
    return match.group(1)


def filename(part: str, version: str) -> str:
    return f"msgkit-{part}-{version}.cdx.json"


def run(cmd: list[str], cwd: Path = ROOT) -> str:
    print("+", " ".join(cmd), flush=True)
    done = subprocess.run(cmd, cwd=cwd, check=True, capture_output=True, text=True)
    return done.stdout


def timestamp() -> str:
    epoch = os.environ.get("SOURCE_DATE_EPOCH")
    moment = datetime.fromtimestamp(int(epoch), tz=timezone.utc) if epoch else datetime.now(tz=timezone.utc)
    return moment.strftime("%Y-%m-%dT%H:%M:%SZ")


def sanitize(text: str) -> str:
    """Cargo package ids carry the absolute path of the checkout (`path+file:///home/runner/...`). Drop it."""
    return re.sub(r"path\+file://[^\"#]*?/crates/", "path+file:///msgkit/crates/", text)


def write(out: Path, part: str, version: str, document: dict[str, Any] | str) -> None:
    text = document if isinstance(document, str) else json.dumps(document, indent=2) + "\n"
    (out / filename(part, version)).write_text(sanitize(text), encoding="utf-8", newline="\n")


def rust(out: Path, version: str) -> None:
    run(["cargo", "cyclonedx", "--format", "json", "--spec-version", SPEC_VERSION, "--describe", "binaries"])
    for part, (crate, produced) in RUST_PARTS.items():
        source = ROOT / "crates" / crate / produced
        if not source.is_file():
            raise SystemExit(f"cargo cyclonedx did not produce {source}")
        document = json.loads(source.read_text(encoding="utf-8"))
        document["metadata"]["timestamp"] = timestamp()
        write(out, part, version, document)
    # cargo-cyclonedx writes next to each manifest; leave the working tree as it was.
    for leftover in (ROOT / "crates").glob("*/*.cdx.json"):
        leftover.unlink()


def node(out: Path, version: str) -> None:
    package = ROOT / "packages" / "node"
    npm = "npm.cmd" if WINDOWS else "npm"
    document = json.loads(
        run([npm, "sbom", "--sbom-format", "cyclonedx", "--omit", "dev", "--package-lock-only"], package)
    )
    pkg = json.loads((package / "package.json").read_text(encoding="utf-8"))
    scope, _, name = pkg["name"].partition("/")
    root = document["metadata"]["component"]
    # npm names the root after the directory when it works from the lock file alone.
    root["name"], root["group"] = name, scope
    bundled = {
        "type": "library",
        "bom-ref": f"msgkit-wasm@{version}",
        "name": "msgkit-wasm",
        "version": version,
        "description": "The WebAssembly module built from this repository and bundled in the package; "
        "its Rust dependencies are listed in the msgkit-wasm SBOM.",
        "licenses": [{"license": {"id": "MIT"}}],
    }
    document.setdefault("components", []).append(bundled)
    for entry in document.get("dependencies", []):
        if entry["ref"] == root["bom-ref"]:
            entry["dependsOn"] = [*entry.get("dependsOn", []), bundled["bom-ref"]]
    document["metadata"]["timestamp"] = timestamp()
    write(out, "node", version, document)


def python(out: Path, version: str) -> None:
    from cyclonedx.model.bom import Bom
    from cyclonedx.model.component import Component, ComponentType
    from cyclonedx.model.license import DisjunctiveLicense
    from cyclonedx.output.json import JsonV1Dot5
    from packageurl import PackageURL

    wheel = Component(
        name="msgkit-core",
        version=version,
        type=ComponentType.LIBRARY,
        bom_ref=f"pkg:pypi/msgkit-core@{version}",
        purl=PackageURL("pypi", name="msgkit-core", version=version),
        licenses=[DisjunctiveLicense(id="MIT")],
        description="The Python wheel: no runtime dependencies; it bundles the native msgkit-ffi library.",
    )
    native = Component(
        name="msgkit-ffi",
        version=version,
        type=ComponentType.LIBRARY,
        bom_ref=f"msgkit-ffi@{version}",
        licenses=[DisjunctiveLicense(id="MIT")],
        description="The native library built from this repository and bundled in the wheel; its Rust "
        "dependencies are listed in the msgkit-ffi SBOM.",
    )
    bom = Bom()
    bom.metadata.component = wheel
    bom.metadata.timestamp = datetime.strptime(timestamp(), "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc)
    bom.components.add(native)
    bom.register_dependency(wheel, [native])
    write(out, "python", version, JsonV1Dot5(bom).output_as_string(indent=2) + "\n")


def kotlin(out: Path, version: str) -> None:
    kotlin_dir = ROOT / "packages" / "kotlin"
    gradlew = str(kotlin_dir / ("gradlew.bat" if WINDOWS else "gradlew"))
    run([gradlew, ":msgkit:cyclonedxDirectBom", ":msgkit-android:cyclonedxDirectBom", "--console=plain"], kotlin_dir)
    for part, module, produced in (
        ("kotlin", "msgkit", "msgkit-kotlin.cdx.json"),
        ("android", "msgkit-android", "msgkit-android.cdx.json"),
    ):
        source = kotlin_dir / module / "build" / "reports" / "cyclonedx" / produced
        document = json.loads(source.read_text(encoding="utf-8"))
        document["metadata"]["timestamp"] = timestamp()
        write(out, part, version, document)


def validate(directory: Path, version: str) -> list[str]:
    """Every expected SBOM exists, is valid CycloneDX 1.5 and describes this version; no local path leaked."""
    from cyclonedx.schema import SchemaVersion
    from cyclonedx.validation.json import JsonStrictValidator

    validator = JsonStrictValidator(SchemaVersion.V1_5)
    problems: list[str] = []
    for part in ALL_PARTS:
        path = directory / filename(part, version)
        if not path.is_file():
            problems.append(f"{path.name}: missing")
            continue
        text = path.read_text(encoding="utf-8")
        error = validator.validate_str(text)
        if error is not None:
            problems.append(f"{path.name}: not valid CycloneDX {SPEC_VERSION}: {error}")
            continue
        document = json.loads(text)
        root = document.get("metadata", {}).get("component", {})
        if document.get("bomFormat") != "CycloneDX" or document.get("specVersion") != SPEC_VERSION:
            problems.append(f"{path.name}: not CycloneDX {SPEC_VERSION}")
        if root.get("version") != version or not root.get("name") or root.get("name") == "node":
            problems.append(f"{path.name}: the root component is {root.get('name')!r} {root.get('version')!r}")
        if part in RUST_PARTS and len(document.get("components", [])) < 5:
            problems.append(f"{path.name}: suspiciously few components")
        if hygiene.HOME_PATH.search(text) and any(
            m.group(1).lower() not in hygiene.HOME_PLACEHOLDERS for m in hygiene.HOME_PATH.finditer(text)
        ):
            problems.append(f"{path.name}: contains a personal home path")
        if re.search(r"path\+file:///(?!msgkit/)", text):
            problems.append(f"{path.name}: contains a local file path")
    return problems


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument(
        "--out", type=Path, help="directory for the SBOMs (created; existing msgkit-*.cdx.json replaced)"
    )
    parser.add_argument("--only", default="", help="comma-separated: rust, node, python, kotlin")
    parser.add_argument("--validate-only", type=Path, metavar="DIR", help="only validate the SBOMs in DIR")
    args = parser.parse_args()
    version = workspace_version()

    if args.validate_only:
        problems = validate(args.validate_only, version)
    else:
        if not args.out:
            parser.error("give --out DIR (or --validate-only DIR)")
        wanted = {p for p in args.only.split(",") if p} or {"rust", "node", "python", "kotlin"}
        unknown = wanted - {"rust", "node", "python", "kotlin"}
        if unknown:
            parser.error(f"unknown part(s): {', '.join(sorted(unknown))}")
        args.out.mkdir(parents=True, exist_ok=True)
        for old in args.out.glob("msgkit-*.cdx.json"):
            old.unlink()
        for name, build in (("rust", rust), ("node", node), ("python", python), ("kotlin", kotlin)):
            if name in wanted:
                build(args.out, version)
        if wanted != {"rust", "node", "python", "kotlin"}:
            print("partial run: not validating the set", file=sys.stderr)
            return
        problems = validate(args.out, version)

    if problems:
        print("SBOM check FAILED:", *problems, sep="\n  ", file=sys.stderr)
        raise SystemExit(1)
    print(f"SBOM check passed ({len(ALL_PARTS)} files, CycloneDX {SPEC_VERSION}, version {version})")


if __name__ == "__main__":
    main()
