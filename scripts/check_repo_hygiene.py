#!/usr/bin/env python3
"""Fails when the repository contains data that looks like a real export or personal data.

msgkit processes personal messages, and the repository is public. This guard runs in CI (and
can run as a pre-commit hook) to keep real exports, databases, secrets and personal identifiers
out of it. Only synthetic data may live under the allow-listed fixture directories.

    python scripts/check_repo_hygiene.py            # tracked files + untracked, non-ignored files
    python scripts/check_repo_hygiene.py --tracked  # only files known to git

Output never contains the offending text: only the rule, the path and the line number, so a CI
log can not itself leak the data it detected.
"""

from __future__ import annotations

import argparse
import fnmatch
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, Iterator

# Directories that may hold *synthetic* sample data (relative, forward slashes, prefix match).
FIXTURE_PREFIXES = (
    "crates/msgkit-core/tests/fixtures/",
    "conformance/",
    "fuzz/corpus/",
)

# The guard's own tests must contain examples of what it detects.
SELF_TEST_PREFIXES = ("scripts/tests/",)

# File names that are never allowed outside the fixture directories.
FORBIDDEN_NAMES = (
    "*.ndjson",
    "*.db",
    "*.db-wal",
    "*.db-shm",
    "*.sqlite",
    "*.sqlite3",
    "_chat.txt",
    "WhatsApp Chat*",
    "message_[0-9]*.json",
    "*.pem",
    "*.p12",
    "*.pfx",
    "*.jks",
    "*.keystore",
    ".env",
    ".env.*",
    "id_rsa",
    "id_ed25519",
)

TEXT_SUFFIXES_SKIPPED = {
    ".png",
    ".jpg",
    ".jpeg",
    ".gif",
    ".ico",
    ".pdf",
    ".jar",
    ".wasm",
    ".so",
    ".dll",
    ".dylib",
    ".zip",
    ".gz",
    ".tgz",
    ".class",
    ".ttf",
    ".woff",
    ".woff2",
}
MAX_TEXT_BYTES = 5 * 1024 * 1024
# Lock files are machine generated and contain no personal data, only hashes and URLs.
SKIPPED_NAMES = {"Cargo.lock", "package-lock.json", "gradle-wrapper.properties"}

# A WhatsApp "Export chat" line: Android "31/12/2023, 21:41 - Name: text" or iOS "[31/12/2023, 21:41:05] Name: text".
WHATSAPP_LINE = re.compile(
    r"^\s*[\u200e\u200f]?\[?\d{1,4}[/.\-]\d{1,2}[/.\-]\d{1,4},?\s+"
    r"\d{1,2}[:.]\d{2}(?::\d{2})?(?:\s?[AaPp]\.?[Mm]\.?)?\]?\s+-?\s*[^:\n]{1,60}: \S"
)
WHATSAPP_LINES_THRESHOLD = 3

EMAIL = re.compile(r"(?<![\w.+-])[A-Za-z0-9._%+-]+@((?:[A-Za-z0-9-]+\.)+[A-Za-z]{2,})\b")
ALLOWED_EMAIL_DOMAINS = (
    "example.com",
    "example.org",
    "example.net",
    "users.noreply.github.com",
    "anthropic.com",
)

# Personal home directories. Generic placeholders (<user>, USERNAME, you...) are fine.
HOME_PATH = re.compile(r"(?:[A-Za-z]:[\\/]+Users[\\/]+|/home/|/Users/)([A-Za-z0-9._-]+)")
HOME_PLACEHOLDERS = {
    "user",
    "username",
    "you",
    "your-name",
    "yourname",
    "name",
    "me",
    "runner",
    "runneradmin",
    "default",
    "public",
    "all",
    "ubuntu",
    "build",
    "ci",
    "vagrant",
    "jdoe",
    "example",
    "x",
    "xxx",
}

# International phone numbers and Sri Lankan national numbers.
PHONE = re.compile(r"(?<![\w.])(\+\d[\d ()-]{7,17}\d|0\d{9})(?!\w)")
# Numbers that are obviously invented: long runs of one digit, sequences, NANP fiction (555-01xx).
SYNTHETIC_FRAGMENTS = ("1234567", "7654321", "0000000", "1111111", "2222222", "5550100", "5550101", "55501")


@dataclass(frozen=True)
class Violation:
    rule: str
    path: str
    line: int  # 0 when the whole file is the problem

    def __str__(self) -> str:
        where = f"{self.path}:{self.line}" if self.line else self.path
        return f"[{self.rule}] {where}"


def is_fixture(rel: str) -> bool:
    return rel.startswith(FIXTURE_PREFIXES + SELF_TEST_PREFIXES)


def looks_synthetic(number: str) -> bool:
    digits = re.sub(r"\D", "", number)
    if any(f in digits for f in SYNTHETIC_FRAGMENTS):
        return True
    return len(set(digits)) <= 2


def scan_text(rel: str, text: str) -> Iterator[Violation]:
    whatsapp_lines = 0
    has_fb_markers = {"participants": False, "timestamp_ms": False, "sender_name": False}
    for number, line in enumerate(text.splitlines(), start=1):
        if WHATSAPP_LINE.match(line):
            whatsapp_lines += 1
            if whatsapp_lines == WHATSAPP_LINES_THRESHOLD:
                yield Violation("looks-like-whatsapp-export", rel, number)
        for marker in has_fb_markers:
            if f'"{marker}"' in line:
                has_fb_markers[marker] = True
        for m in EMAIL.finditer(line):
            if not m.group(1).lower().endswith(ALLOWED_EMAIL_DOMAINS):
                yield Violation("email-address", rel, number)
        for m in HOME_PATH.finditer(line):
            if m.group(1).lower() not in HOME_PLACEHOLDERS:
                yield Violation("personal-home-path", rel, number)
        for m in PHONE.finditer(line):
            if not looks_synthetic(m.group(1)):
                yield Violation("phone-number", rel, number)
    if rel.endswith((".json", ".jsonl")) and all(has_fb_markers.values()):
        yield Violation("looks-like-messenger-export", rel, 0)


def scan_file(root: Path, rel: str) -> Iterator[Violation]:
    """Scans one repository-relative path (forward slashes)."""
    if is_fixture(rel):
        return
    name = rel.rsplit("/", 1)[-1]
    if any(fnmatch.fnmatch(name, pattern) for pattern in FORBIDDEN_NAMES):
        yield Violation("forbidden-file-name", rel, 0)
        return
    path = root / rel
    if name in SKIPPED_NAMES or path.suffix.lower() in TEXT_SUFFIXES_SKIPPED:
        return
    try:
        if path.stat().st_size > MAX_TEXT_BYTES:
            yield Violation("file-too-large-to-review", rel, 0)
            return
        text = path.read_bytes().decode("utf-8", errors="replace")
    except OSError:
        return
    yield from scan_text(rel, text)


def git_files(root: Path, tracked_only: bool) -> list[str]:
    cmd = ["git", "ls-files", "-z"]
    if not tracked_only:
        cmd += ["--cached", "--others", "--exclude-standard"]
    out = subprocess.run(cmd, cwd=root, check=True, capture_output=True).stdout
    return sorted({p for p in out.decode("utf-8").split("\0") if p})


def scan(root: Path, files: Iterable[str]) -> list[Violation]:
    found: list[Violation] = []
    for rel in files:
        found.extend(scan_file(root, rel))
    return found


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--root", default=".", help="repository root (default: current directory)")
    parser.add_argument("--tracked", action="store_true", help="check only files tracked by git")
    args = parser.parse_args(argv)

    root = Path(args.root).resolve()
    files = git_files(root, args.tracked)
    violations = scan(root, files)
    if violations:
        print(f"Repository hygiene check FAILED: {len(violations)} finding(s). Text is intentionally not printed.")
        for v in violations:
            print(" ", v)
        print("Real exports and personal data must never be committed. Use synthetic data under a fixture directory.")
        return 1
    print(f"Repository hygiene check passed ({len(files)} files scanned).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
