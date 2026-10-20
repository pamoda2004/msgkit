"""Loads the msgkit native library and wraps its three C functions."""

from __future__ import annotations

import ctypes
import json
import os
import sys
import threading
import warnings
from pathlib import Path
from typing import Any, Optional

from .errors import MsgkitError

_lock = threading.Lock()
_lib: Optional[ctypes.CDLL] = None


def library_filename() -> str:
    platform = sys.platform  # (a plain variable keeps type checkers from pruning branches per host OS)
    if platform == "win32":
        return "msgkit_ffi.dll"
    if platform == "darwin":
        return "libmsgkit_ffi.dylib"
    return "libmsgkit_ffi.so"


def _source_checkout_root() -> Optional[Path]:
    """The repository root when this package is imported from a source checkout, otherwise ``None``.

    An installed package (a wheel in ``site-packages``) is never a source checkout, so it never looks for a
    development build: the directory five levels above an installed file is not a place to load code from.
    """
    here = Path(__file__).resolve()
    if len(here.parents) < 5:
        return None
    root = here.parents[4]
    expected = (root / "packages" / "python" / "src" / "msgkit").resolve()
    if here.parent == expected and (root / "Cargo.toml").is_file() and (root / "crates" / "msgkit-ffi").is_dir():
        return root
    return None


def _bundled() -> Path:
    return Path(__file__).resolve().parent / "_lib" / library_filename()


def _candidates() -> list[Path]:
    """The libraries to try, in order: the one bundled in the wheel, then (source checkout only) a development build."""
    name = library_filename()
    found = [_bundled()]
    root = _source_checkout_root()
    if root is not None:
        for profile in ("release", "debug"):
            found.append(root / "target" / profile / name)
    return found


def _explicit_override() -> Optional[Path]:
    """``MSGKIT_LIB``, checked: it must be an absolute path to an existing file.

    It keeps priority over the bundled library, because it is an explicit choice. A wrong value is an error and
    never falls back silently, so nobody believes they are running their own build when they are not.
    """
    raw = os.environ.get("MSGKIT_LIB")
    if not raw:
        return None
    path = Path(raw)
    if not path.is_absolute():
        raise FileNotFoundError("MSGKIT_LIB must be an absolute path to the native library")
    if not path.is_file():
        raise FileNotFoundError("MSGKIT_LIB does not name an existing file")
    return path


def _check_version(lib: ctypes.CDLL) -> None:
    """The loaded library must be the version of this package: a stale or foreign library is refused."""
    from . import __version__

    native = str(lib.msgkit_version().decode("ascii"))
    if native != __version__:
        raise RuntimeError(
            f"the msgkit native library is version {native} but the Python package is version {__version__}: "
            "refusing to use a mismatched library"
        )


def _open(path: Path) -> ctypes.CDLL:
    lib = ctypes.CDLL(str(path))
    lib.msgkit_call.argtypes = [ctypes.c_char_p, ctypes.c_char_p]
    # c_void_p (not c_char_p): Python must not copy-and-forget the
    # pointer that has to be handed back to msgkit_free.
    lib.msgkit_call.restype = ctypes.c_void_p
    lib.msgkit_free.argtypes = [ctypes.c_void_p]
    lib.msgkit_free.restype = None
    lib.msgkit_version.argtypes = []
    lib.msgkit_version.restype = ctypes.c_char_p
    _check_version(lib)
    return lib


def _load() -> ctypes.CDLL:
    global _lib
    with _lock:
        if _lib is not None:
            return _lib
        override = _explicit_override()
        if override is not None:
            if _bundled().is_file():
                warnings.warn(
                    "MSGKIT_LIB replaces the native library bundled with the msgkit wheel; "
                    "unset it to use the bundled, release-verified library",
                    RuntimeWarning,
                    stacklevel=3,
                )
            _lib = _open(override)
            return _lib
        for path in _candidates():
            if path.is_file():
                _lib = _open(path)
                return _lib
        raise FileNotFoundError(
            f"msgkit native library '{library_filename()}' not found in the installed package. "
            "Reinstall msgkit (a wheel bundles it), set MSGKIT_LIB to an absolute path, or, in a source "
            "checkout, build it with `cargo build -p msgkit-ffi --release`."
        )


def native_version() -> str:
    return str(_load().msgkit_version().decode("ascii"))


def dispatch(op: str, payload: Any = None) -> Any:
    """Runs ``op`` and returns the ``data`` of the response envelope.

    Raises :class:`MsgkitError` for error envelopes. Thread-safe.
    """
    lib = _load()
    raw = lib.msgkit_call(
        op.encode("utf-8"), json.dumps({} if payload is None else payload, separators=(",", ":")).encode("utf-8")
    )
    if not raw:
        raise MsgkitError("internal_error", "msgkit returned no result")
    try:
        envelope = json.loads(ctypes.string_at(raw).decode("utf-8"))
    finally:
        lib.msgkit_free(raw)
    if not envelope["ok"]:
        error = envelope["error"]
        raise MsgkitError(error["code"], error["message"])
    return envelope["data"]
