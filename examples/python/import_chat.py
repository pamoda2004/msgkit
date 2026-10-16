"""Imports a WhatsApp chat export with the msgkit Python package and prints a summary.

    cargo build -p msgkit-ffi
    python examples/python/import_chat.py "C:\\chats\\chat.txt" --self-name "Alice" --utc-offset 330

Only counts and ids are printed, never message text, so the output is safe to share.
The script runs straight from a repository checkout (no pip install needed).
"""

import argparse
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "packages" / "python" / "src"))

import msgkit  # noqa: E402


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("chat", help="WhatsApp 'Export chat' .txt file")
    parser.add_argument("--self-name", help="your name as shown in the chat (sets message direction)")
    parser.add_argument("--utc-offset", type=int, default=0, help="phone's UTC offset in minutes (330 = UTC+05:30)")
    parser.add_argument("--date-order", choices=["auto", "dmy", "mdy"], default="auto")
    args = parser.parse_args()

    text = Path(args.chat).read_text(encoding="utf-8-sig")
    report = msgkit.parse_whatsapp_export(
        text,
        self_name=args.self_name,
        utc_offset_minutes=args.utc_offset,
        date_order=args.date_order,
    )

    events = report["events"]
    print(f"msgkit {msgkit.version()['crate_version']} (schema {msgkit.version()['schema_version']})")
    print(f"events:   {len(events)}")
    print(f"warnings: {len(report['warnings'])}")
    for warning in report["warnings"]:
        print("  -", warning)
    for (kind, direction), n in sorted(Counter((e["type"], e["direction"]) for e in events).items()):
        print(f"  {kind:8} {direction:9} {n}")
    if events:
        print("first:", events[0]["timestamp"], events[0]["id"])
        print("last: ", events[-1]["timestamp"], events[-1]["id"])


if __name__ == "__main__":
    main()
