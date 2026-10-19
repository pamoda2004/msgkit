import hashlib
import hmac
import json
import threading
import unittest
from pathlib import Path
from typing import Any, List

import msgkit

FIXTURES = Path(__file__).resolve().parents[3] / "crates" / "msgkit-core" / "tests" / "fixtures"


def fixture(name: str) -> str:
    return (FIXTURES / name).read_text(encoding="utf-8")


def messages(report: Any) -> List[Any]:
    return [e for e in report["events"] if e["type"] == "message"]


def calls(report: Any) -> List[Any]:
    return [e for e in report["events"] if e["type"] == "call"]


class VersionTests(unittest.TestCase):
    def test_versions_agree(self) -> None:
        info = msgkit.version()
        self.assertEqual(info["schema_version"], "1.0.0")
        self.assertEqual(info["crate_version"], msgkit.__version__)
        self.assertEqual(msgkit.native_version(), msgkit.__version__)

    def test_package_version_matches_the_rust_workspace(self) -> None:
        cargo = (Path(__file__).resolve().parents[3] / "Cargo.toml").read_text(encoding="utf-8")
        line = next(row for row in cargo.splitlines() if row.startswith("version = "))
        self.assertEqual(line.split('"')[1], msgkit.__version__)


class WhatsAppExportTests(unittest.TestCase):
    def test_android_day_first(self) -> None:
        r = msgkit.parse_whatsapp_export(fixture("whatsapp_android_dmy.txt"))
        self.assertEqual(r["warnings"], [])
        self.assertEqual(len(r["events"]), 8)
        self.assertEqual(len(messages(r)), 7)
        self.assertEqual(calls(r)[0]["outcome"], "missed")
        self.assertEqual(r["events"][0]["timestamp"], "2023-12-31T21:41:00Z")
        oks = [m for m in messages(r) if m.get("body") == "ok"]
        self.assertEqual(len(oks), 2)
        self.assertNotEqual(oks[0]["id"], oks[1]["id"])

    def test_keyword_options_are_applied(self) -> None:
        r = msgkit.parse_whatsapp_export(fixture("whatsapp_android_dmy.txt"), self_name="alice", utc_offset_minutes=330)
        self.assertEqual(r["events"][0]["timestamp"], "2023-12-31T16:11:00Z")
        self.assertEqual(r["events"][0]["direction"], "outbound")
        self.assertEqual(r["events"][1]["direction"], "inbound")

    def test_explicit_date_order(self) -> None:
        text = "03/04/2024, 10:00 - A: x\n"
        auto = msgkit.parse_whatsapp_export(text)
        self.assertTrue(any("ambiguous" in w for w in auto["warnings"]))
        self.assertEqual(auto["events"][0]["timestamp"], "2024-04-03T10:00:00Z")
        mdy = msgkit.parse_whatsapp_export(text, date_order="mdy")
        self.assertEqual(mdy["warnings"], [])
        self.assertEqual(mdy["events"][0]["timestamp"], "2024-03-04T10:00:00Z")

    def test_unicode_round_trip(self) -> None:
        body = 'සිංහල 🎉 café "quoted" \\ back\\slash'
        r = msgkit.parse_whatsapp_export(f"01/02/2024, 10:00 - සමන්: {body}\n")
        first: Any = r["events"][0]
        self.assertEqual(first["body"], body)
        self.assertEqual(first["sender"]["id"], "සමන්")

    def test_bom_is_tolerated(self) -> None:
        r = msgkit.parse_whatsapp_export("\ufeff01/02/2024, 10:00 - A: hi\n")
        self.assertEqual(len(r["events"]), 1)

    def test_reimport_gives_identical_ids(self) -> None:
        text = fixture("whatsapp_android_dmy.txt")
        self.assertEqual(msgkit.parse_whatsapp_export(text), msgkit.parse_whatsapp_export(text))

    def test_garbage_is_a_warning_not_an_exception(self) -> None:
        r = msgkit.parse_whatsapp_export("not a chat export")
        self.assertEqual(r["events"], [])
        self.assertTrue(r["warnings"])

    def test_large_input(self) -> None:
        line = "31/12/2023, 21:41 - Alice: some reasonably long message body to make the file big\n"
        r = msgkit.parse_whatsapp_export(line * 50_000)
        self.assertEqual(len(r["events"]), 50_000)
        self.assertEqual(len({e["id"] for e in r["events"]}), 50_000)


class OtherSourceTests(unittest.TestCase):
    def test_facebook_object_and_string(self) -> None:
        text = fixture("facebook_thread.json")
        a = msgkit.parse_facebook_export(json.loads(text), self_name="Alice Perera")
        b = msgkit.parse_facebook_export(text, self_name="Alice Perera")
        self.assertEqual(a, b)
        self.assertEqual(len(a["events"]), 5)
        self.assertEqual(messages(a)[2]["body"], "café ❤")
        self.assertEqual(messages(a)[2]["direction"], "outbound")
        self.assertEqual(calls(a)[1]["duration_secs"], 125)

    def test_device_batch(self) -> None:
        batch = json.loads(fixture("device_batch.json"))
        r = msgkit.normalize_device(batch, own_number="+94710000000", default_country_code="94")
        self.assertEqual(len(messages(r)), 3)
        self.assertEqual(messages(r)[1]["recipients"][0]["id"], "+94700000000")
        self.assertEqual([c["outcome"] for c in calls(r)], ["answered", "missed", "answered", "rejected"])

    def test_webhooks(self) -> None:
        wa = msgkit.normalize_whatsapp_webhook(json.loads(fixture("whatsapp_webhook.json")))
        self.assertEqual(len(messages(wa)), 3)
        self.assertEqual(messages(wa)[0]["external_id"], "wamid.AAA")
        ms = msgkit.normalize_messenger_webhook(json.loads(fixture("messenger_webhook.json")))
        self.assertEqual([m["direction"] for m in messages(ms)], ["inbound", "outbound"])

    def test_signature_verification_matches_hmac(self) -> None:
        body = fixture("whatsapp_webhook.json")
        sig = "sha256=" + hmac.new(b"app-secret", body.encode(), hashlib.sha256).hexdigest()
        self.assertTrue(msgkit.verify_signature(secret="app-secret", body=body, signature=sig))
        self.assertTrue(msgkit.verify_signature(secret="app-secret", body=body.encode(), signature=sig))
        self.assertFalse(msgkit.verify_signature(secret="wrong", body=body, signature=sig))
        self.assertFalse(msgkit.verify_signature(secret="app-secret", body=body + " ", signature=sig))
        self.assertFalse(msgkit.verify_signature(secret="app-secret", body=body, signature="sha256=zz"))


class ErrorTests(unittest.TestCase):
    def test_error_codes(self) -> None:
        with self.assertRaises(msgkit.MsgkitError) as ctx:
            msgkit.dispatch("nope")
        self.assertEqual(ctx.exception.code, "unknown_op")
        with self.assertRaises(msgkit.MsgkitError) as ctx:
            msgkit.parse_facebook_export("[1, 2]")
        self.assertEqual(ctx.exception.code, "parse_error")
        with self.assertRaises(msgkit.MsgkitError) as ctx:
            msgkit.normalize_messenger_webhook({"object": "whatsapp_business_account"})
        self.assertEqual(ctx.exception.code, "parse_error")


class ConcurrencyTests(unittest.TestCase):
    def test_threads_share_the_library_safely(self) -> None:
        text = fixture("whatsapp_android_dmy.txt")
        expected = msgkit.parse_whatsapp_export(text)
        failures = []

        def work() -> None:
            try:
                for _ in range(100):
                    if msgkit.parse_whatsapp_export(text) != expected:
                        failures.append("mismatch")
            except Exception as e:
                failures.append(repr(e))

        threads = [threading.Thread(target=work) for _ in range(8)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        self.assertEqual(failures, [])


if __name__ == "__main__":
    unittest.main()
