#!/usr/bin/env python3
"""Prints the least-covered source files of an lcov report as GitHub annotations.

    python scripts/coverage_gaps.py lcov.info [--limit 8]

Only file names, line numbers and percentages are printed, never source text.
"""

from __future__ import annotations

import argparse
from pathlib import Path


def ranges(lines: list[int]) -> str:
    out: list[str] = []
    start = prev = None
    for n in sorted(lines):
        if start is None:
            start = prev = n
        elif n == prev + 1:  # type: ignore[operator]
            prev = n
        else:
            out.append(str(start) if start == prev else f"{start}-{prev}")
            start = prev = n
    if start is not None:
        out.append(str(start) if start == prev else f"{start}-{prev}")
    return ",".join(out)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("lcov")
    parser.add_argument("--limit", type=int, default=8)
    args = parser.parse_args()

    files: dict[str, tuple[int, list[int]]] = {}
    current = ""
    total = 0
    missed: list[int] = []
    for raw in Path(args.lcov).read_text(encoding="utf-8").splitlines():
        if raw.startswith("SF:"):
            current, total, missed = raw[3:].replace("\\", "/"), 0, []
        elif raw.startswith("DA:"):
            number, hits = raw[3:].split(",")[:2]
            total += 1
            if int(hits) == 0:
                missed.append(int(number))
        elif raw == "end_of_record" and current:
            files[current] = (total, missed)
    worst = sorted(files.items(), key=lambda kv: -len(kv[1][1]))[: args.limit]
    for name, (count, lines) in worst:
        if lines:
            short = name.split("/crates/")[-1]
            print(f"::notice title=uncovered {short}::{len(lines)} of {count} lines uncovered: {ranges(lines)[:600]}")


if __name__ == "__main__":
    main()
