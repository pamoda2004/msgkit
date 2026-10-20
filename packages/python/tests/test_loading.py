"""How the native library is found: the bundled library first, a development build only in a source checkout,
and ``MSGKIT_LIB`` as a checked, explicit override. Each case runs in a fresh interpreter because the loaded
library is cached for the life of the process."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import msgkit
from msgkit import _native

SRC = Path(msgkit.__file__).resolve().parent


def _real_library() -> Path:
    for candidate in _native._candidates():
        if candidate.is_file():
            return candidate
    raise unittest.SkipTest("no native library to test with (cargo build -p msgkit-ffi)")


def _run(code: str, site: Path, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    """Runs ``code`` in a fresh interpreter that imports msgkit from ``site`` and nowhere else."""
    full_env = {k: v for k, v in os.environ.items() if k not in ("MSGKIT_LIB", "PYTHONPATH")}
    full_env["PYTHONPATH"] = str(site)
    full_env.update(env or {})
    return subprocess.run(  # noqa: S603 (our own interpreter and our own code)
        [sys.executable, "-c", code], capture_output=True, text=True, env=full_env, cwd=str(site.parent), check=False
    )


class InstalledLayout:
    """A copy of the package laid out like an installed wheel: ``<venv>/lib/python3/site-packages/msgkit``."""

    def __init__(self, bundled: bool) -> None:
        self.tmp = tempfile.TemporaryDirectory(prefix="msgkit-load-")
        self.venv = Path(self.tmp.name) / "venv"
        self.site = self.venv / "lib" / "python3" / "site-packages"
        self.package = self.site / "msgkit"
        shutil.copytree(SRC, self.package, ignore=shutil.ignore_patterns("__pycache__", "_lib"))
        self.lib = _real_library()
        if bundled:
            (self.package / "_lib").mkdir()
            shutil.copy(self.lib, self.package / "_lib" / self.lib.name)

    def plant_decoy(self) -> Path:
        """A library where the old development fallback looked for one: ``<venv>/target/release``."""
        decoy = self.venv / "target" / "release" / self.lib.name
        decoy.parent.mkdir(parents=True)
        shutil.copy(self.lib, decoy)
        return decoy

    def close(self) -> None:
        self.tmp.cleanup()


class LoadingTests(unittest.TestCase):
    def layout(self, bundled: bool) -> InstalledLayout:
        layout = InstalledLayout(bundled)
        self.addCleanup(layout.close)
        return layout

    def test_an_installed_copy_is_not_a_source_checkout(self) -> None:
        layout = self.layout(bundled=False)
        result = _run("from msgkit import _native; print(_native._source_checkout_root())", layout.site)
        self.assertEqual(result.stdout.strip(), "None", result.stderr)

    def test_an_installed_package_never_loads_a_library_from_a_target_directory(self) -> None:
        layout = self.layout(bundled=False)
        layout.plant_decoy()
        result = _run("import msgkit; msgkit.native_version()", layout.site)
        self.assertNotEqual(result.returncode, 0, "the decoy library must not be loaded")
        self.assertIn("not found in the installed package", result.stderr)
        message = result.stderr.strip().splitlines()[-1]  # the traceback lines above name source files
        self.assertNotIn("target", message, "the error must not list guessed paths")

    def test_the_bundled_library_is_used_and_works(self) -> None:
        layout = self.layout(bundled=True)
        layout.plant_decoy()
        code = "import msgkit; print(msgkit.native_version()); print(msgkit.version()['crate_version'])"
        result = _run(code, layout.site)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.split(), [msgkit.__version__, msgkit.__version__])

    def test_an_override_must_be_an_absolute_path_to_an_existing_file(self) -> None:
        layout = self.layout(bundled=True)
        for value, message in (
            ("relative/libmsgkit_ffi.so", "absolute path"),
            (str(layout.venv / "nothing.so"), "existing file"),
        ):
            result = _run("import msgkit; msgkit.native_version()", layout.site, {"MSGKIT_LIB": value})
            self.assertNotEqual(result.returncode, 0, value)
            self.assertIn(message, result.stderr)
            self.assertIn("FileNotFoundError", result.stderr, "a bad override must not fall back silently")

    def test_an_override_wins_over_the_bundled_library_with_a_warning(self) -> None:
        layout = self.layout(bundled=True)
        other = layout.venv / "mine" / layout.lib.name
        other.parent.mkdir(parents=True)
        shutil.copy(layout.lib, other)
        code = (
            "import warnings, json\n"
            "with warnings.catch_warnings(record=True) as caught:\n"
            "    warnings.simplefilter('always')\n"
            "    import msgkit\n"
            "    msgkit.native_version()\n"
            "print(json.dumps([str(w.category.__name__) + ': ' + str(w.message) for w in caught]))\n"
        )
        result = _run(code, layout.site, {"MSGKIT_LIB": str(other)})
        self.assertEqual(result.returncode, 0, result.stderr)
        warned = json.loads(result.stdout.strip().splitlines()[-1])
        self.assertEqual(len(warned), 1, warned)
        self.assertTrue(warned[0].startswith("RuntimeWarning: MSGKIT_LIB replaces the native library bundled"), warned)

    def test_an_override_without_a_bundled_library_is_quiet(self) -> None:
        layout = self.layout(bundled=False)
        code = "import msgkit; print(msgkit.native_version())"
        result = _run(code, layout.site, {"MSGKIT_LIB": str(layout.lib)})
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), msgkit.__version__)
        self.assertNotIn("RuntimeWarning", result.stderr)

    def test_a_library_of_another_version_is_refused(self) -> None:
        layout = self.layout(bundled=True)
        code = "import msgkit; msgkit.__version__ = '9.9.9'; msgkit.native_version()"
        result = _run(code, layout.site)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("refusing to use a mismatched library", result.stderr)
        self.assertIn("9.9.9", result.stderr)

    @unittest.skipIf(_native._source_checkout_root() is None, "run from the installed wheel, not a checkout")
    def test_a_source_checkout_still_finds_the_development_build(self) -> None:
        # The tests of this repository rely on it: PYTHONPATH=packages/python/src and no environment variable.
        self.assertIsNotNone(_native._source_checkout_root())
        result = _run("import msgkit; print(msgkit.native_version())", SRC.parent)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), msgkit.__version__)


if __name__ == "__main__":
    unittest.main()
