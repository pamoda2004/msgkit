import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import check_repo_hygiene as hygiene


def rules(rel: str, text: str | None = None, root: Path | None = None) -> set[str]:
    """Returns the rule ids that fire for `rel` (creating it with `text` when given)."""
    with tempfile.TemporaryDirectory() as tmp:
        base = root or Path(tmp)
        if text is not None:
            target = base / rel
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(text, encoding="utf-8")
        return {v.rule for v in hygiene.scan_file(base, rel)}


WHATSAPP = (
    "31/12/2023, 21:41 - Someone: first line\n"
    "31/12/2023, 21:42 - Other: second line\n"
    "31/12/2023, 21:43 - Someone: third line\n"
)
IOS = (
    "[31/12/2023, 21:41:05] Someone: first\n"
    "[31/12/2023, 21:42:10] Other: second\n"
    "[31/12/2023, 21:43:20] Someone: third\n"
)
MESSENGER = '{"participants": [], "messages": [{"sender_name": "A", "timestamp_ms": 1}]}'


class ForbiddenNames(unittest.TestCase):
    def test_database_and_export_file_names_are_rejected(self) -> None:
        for name in [
            "events.db",
            "x/events.db-wal",
            "out.ndjson",
            "_chat.txt",
            "WhatsApp Chat with A.txt",
            "inbox/message_1.json",
            "keys/server.pem",
            ".env",
            "a/b/id_rsa",
            "release.keystore",
        ]:
            with self.subTest(name=name):
                self.assertIn("forbidden-file-name", rules(name, "x"))

    def test_normal_names_are_fine(self) -> None:
        for name in ["README.md", "src/message.rs", "docs/chat-design.md", "messages.json"]:
            with self.subTest(name=name):
                self.assertEqual(rules(name, "hello"), set())


class ExportShapes(unittest.TestCase):
    def test_whatsapp_android_and_ios_shapes(self) -> None:
        self.assertIn("looks-like-whatsapp-export", rules("notes/sample.txt", WHATSAPP))
        self.assertIn("looks-like-whatsapp-export", rules("notes/sample2.txt", IOS))

    def test_two_lines_are_not_enough(self) -> None:
        self.assertNotIn("looks-like-whatsapp-export", rules("docs/x.md", "\n".join(WHATSAPP.splitlines()[:2])))

    def test_code_that_merely_mentions_the_format_is_fine(self) -> None:
        code = 'let text = "31/12/2023, 21:41 - Alice: hi\\n";\n' * 5
        self.assertNotIn("looks-like-whatsapp-export", rules("src/lib.rs", code))

    def test_messenger_shape(self) -> None:
        self.assertIn("looks-like-messenger-export", rules("data.json", MESSENGER))

    def test_fixture_directories_are_exempt(self) -> None:
        self.assertEqual(rules("crates/msgkit-core/tests/fixtures/chat.txt", WHATSAPP), set())
        self.assertEqual(rules("conformance/cases/a.json", MESSENGER), set())
        self.assertEqual(rules("conformance/events.ndjson", "{}"), set())


class PersonalIdentifiers(unittest.TestCase):
    def test_email_addresses(self) -> None:
        self.assertIn("email-address", rules("a.md", "contact: someone@" + "gmail" + ".com"))
        self.assertEqual(rules("a.md", "noreply: 1+me@users.noreply.github.com or a@example.com"), set())

    def test_home_paths(self) -> None:
        self.assertIn("personal-home-path", rules("a.md", r"C:\Users\realname\Documents"))
        self.assertIn("personal-home-path", rules("a.md", "/home/realname/project"))
        self.assertEqual(rules("a.md", r"C:\Users\<user>\x and /home/runner/work and C:\Users\you\y"), set())

    def test_phone_numbers(self) -> None:
        self.assertIn("phone-number", rules("a.md", "call +44 7700 900123 now"))
        self.assertIn("phone-number", rules("a.md", "call 0768429137 now"))

    def test_obviously_synthetic_numbers_are_allowed(self) -> None:
        for text in ["+94771234567", "0771234567", "+94710000000", "+12025550123", "+15555550100", "+11111111111"]:
            with self.subTest(text=text):
                self.assertNotIn("phone-number", rules("a.md", text))

    def test_ids_and_timestamps_are_not_phone_numbers(self) -> None:
        self.assertEqual(rules("a.rs", "let t = 1704067200000; // 2024-01-01 abcdef0123456789abcdef0123456789"), set())


class Behaviour(unittest.TestCase):
    def test_output_never_contains_the_offending_text(self) -> None:
        secret = "someone.private@" + "gmail" + ".com"
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "a.md").write_text(secret, encoding="utf-8")
            v = hygiene.scan(root, ["a.md"])
        self.assertTrue(v)
        self.assertNotIn(secret, "\n".join(str(x) for x in v))

    def test_binary_and_missing_files_do_not_crash(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "img.png").write_bytes(b"\x89PNG\x00\xff")
            (root / "blob.dat").write_bytes(bytes(range(256)))
            self.assertEqual(hygiene.scan(root, ["img.png", "blob.dat", "missing.txt"]), [])


if __name__ == "__main__":
    unittest.main()
