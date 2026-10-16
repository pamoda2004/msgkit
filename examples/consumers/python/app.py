"""A tiny application that uses msgkit as an *installed dependency* (not from source).

Prints chat statistics; the same program exists for Node.js, Kotlin and Rust and prints the same output.
"""

import hashlib
import hmac
import sys
from collections import Counter
from pathlib import Path

import msgkit

default = Path(__file__).resolve().parents[3] / "crates" / "msgkit-core" / "tests" / "fixtures" / "whatsapp_android_dmy.txt"
path = Path(sys.argv[1]) if len(sys.argv) > 1 else default
report = msgkit.parse_whatsapp_export(path.read_text(encoding="utf-8-sig"), self_name="Alice", utc_offset_minutes=330)

events = report["events"]
messages = [e for e in events if e["type"] == "message"]
calls = [e for e in events if e["type"] == "call"]
per_sender = Counter(m["sender"]["id"] for m in messages)

print(f"events: {len(events)}")
print(f"messages: {len(messages)}, calls: {len(calls)}")
print("per sender:")
for name, n in sorted(per_sender.items()):
    print(f"  {name}: {n}")
print(f"attachments: {sum(len(m['attachments']) for m in messages)}")
print(f"missed calls: {sum(1 for c in calls if c['outcome'] == 'missed')}")
print(f"first: {events[0]['timestamp']}  last: {events[-1]['timestamp']}")

# Webhook signature check with a signature computed independently by the language's own crypto library.
body = '{"object":"page","entry":[]}'
good = "sha256=" + hmac.new(b"app-secret", body.encode(), hashlib.sha256).hexdigest()
print(f"signature valid: {str(msgkit.verify_signature(secret='app-secret', body=body, signature=good)).lower()}")
print(f"tampered body valid: {str(msgkit.verify_signature(secret='app-secret', body=body + ' ', signature=good)).lower()}")
