"""Loads the msgkit native library and wraps its three C functions."""

from __future__ import annotations

import ctypes
import json
import os
import sys
import threading
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


def _candidates() -> list[Path]:
    name = library_filename()
    found: list[Path] = []
    explicit = os.environ.get("MSGKIT_LIB")
    if explicit:
        found.append(Path(explicit))
    # 1. The library bundled in the wheel.
    found.append(Path(__file__).resolve().parent / "_lib" / name)
    # 2. A development checkout: packages/python/src/msgkit -> repository root.
    root = Path(__file__).resolve().parents[4]
    for profile in ("release", "debug"):
        found.append(root / "target" / profile / name)
    return found


def _load() -> ctypes.CDLL:
    global _lib
    with _lock:
        if _lib is not None:
            return _lib
        for path in _candidates():
            if path.is_file():
                lib = ctypes.CDLL(str(path))
                lib.msgkit_call.argtypes = [ctypes.c_char_p, ctypes.c_char_p]
                # c_void_p (not c_char_p): Python must not copy-and-forget the
                # pointer that has to be handed back to msgkit_free.
                lib.msgkit_call.restype = ctypes.c_void_p
                lib.msgkit_free.argtypes = [ctypes.c_void_p]
                lib.msgkit_free.restype = None
                lib.msgkit_version.argtypes = []
                lib.msgkit_version.restype = ctypes.c_char_p
                _lib = lib
                return lib
        tried = "\n  ".join(str(p) for p in _candidates())
        raise FileNotFoundError(
            f"msgkit native library '{library_filename()}' not found. Set MSGKIT_LIB or build it "
            f"with `cargo build -p msgkit-ffi --release`. Looked in:\n  {tried}"
        )


def native_version() -> str:
    return str(_load().msgkit_version().decode("ascii"))


def dispatch(op: str, payload: Any = None) -> Any:
    """Runs ``op`` and returns the ``data`` of the response envelope.

    Raises :class:`MsgkitError` for error envelopes. Thread-safe.
    """
    lib = _load()
    raw = lib.msgkit_call(op.encode("utf-8"), json.dumps({} if payload is None else payload).encode("utf-8"))
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
