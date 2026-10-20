"""Guards on .github/workflows/release.yml that must never be removed.

The release workflow can be started by hand (`workflow_dispatch`, a dry run). A dry run must never create a GitHub
release, publish to PyPI or npm, or store attestations, so each of those steps is guarded by a condition that is
true only for a push of a `v*` tag. These tests read the workflow as text (no YAML library is needed) and fail if a
guard is removed, if a new publishing step appears outside a guarded job, or if a permission grows.
"""

import re
import unittest
from pathlib import Path

WORKFLOW = Path(__file__).resolve().parents[2] / ".github" / "workflows" / "release.yml"

# A selected tag with `workflow_dispatch` also has github.ref = refs/tags/v..., so the event name matters too.
GUARD_EVENT = "github.event_name == 'push'"
GUARD_TAG = "startsWith(github.ref, 'refs/tags/v')"

# Jobs that publish or release: only for a pushed v* tag.
GUARDED_JOBS = {"github-release", "publish-pypi", "publish-npm"}
# Jobs allowed to write: job -> permissions that exceed `contents: read`.
ALLOWED_WRITES = {
    "github-release": {"contents: write"},
    "publish-pypi": {"id-token: write"},
    "publish-npm": {"id-token: write"},
    "sign-and-attest": {"id-token: write", "attestations: write", "artifact-metadata: write"},
}
# Anything that publishes, and the jobs in which it may appear.
PUBLISHING = {
    "softprops/action-gh-release": {"github-release"},
    "pypa/gh-action-pypi-publish": {"publish-pypi"},
    "npm publish": {"publish-npm"},
    "gh release": set(),
    "gh api": set(),
    "twine upload": set(),
    "cargo publish": set(),
    "actions/attest": {"sign-and-attest"},
}


def text() -> str:
    return WORKFLOW.read_text(encoding="utf-8").replace("\r\n", "\n")


def jobs() -> dict[str, str]:
    """The text of each job, by name."""
    body = text().split("\njobs:\n", 1)[1]
    parts = re.split(r"(?m)^  ([a-z][a-z0-9-]*):\n", body)
    return {parts[i]: parts[i + 1] for i in range(1, len(parts), 2)}


def job_level(block: str, key: str) -> list[str]:
    """Lines of a job's own `key:` (four spaces of indent), not those of its steps."""
    return re.findall(rf"(?m)^    {re.escape(key)}:(.*)$", block)


class ReleaseWorkflowGuards(unittest.TestCase):
    def test_a_dry_run_exists_and_a_tag_push_still_triggers_a_release(self) -> None:
        head = text().split("\njobs:\n", 1)[0]
        self.assertIn("workflow_dispatch:", head)
        self.assertRegex(head, r'(?s)push:\s+tags: \["v\*"\]')

    def test_the_workflow_level_permissions_are_read_only(self) -> None:
        head = text().split("\njobs:\n", 1)[0]
        block = re.search(r"(?m)^permissions:\n((?:  .*\n)+)", head)
        self.assertIsNotNone(block, "no workflow-level permissions block")
        assert block is not None
        self.assertEqual(block.group(1).strip(), "contents: read")

    def test_publishing_jobs_run_only_for_a_pushed_version_tag(self) -> None:
        all_jobs = jobs()
        for name in GUARDED_JOBS:
            self.assertIn(name, all_jobs, f"job {name} is missing")
            conditions = job_level(all_jobs[name], "if")
            self.assertEqual(len(conditions), 1, f"{name} needs exactly one job-level `if`")
            condition = conditions[0]
            self.assertIn(GUARD_EVENT, condition, f"{name}: the guard must require a push, not a manual run")
            self.assertIn(GUARD_TAG, condition, f"{name}: the guard must require a v* tag")
            self.assertNotIn("||", condition, f"{name}: the guard must not be weakened with `||`")

    def test_publishing_steps_appear_only_in_their_own_guarded_jobs(self) -> None:
        for name, block in jobs().items():
            for needle, allowed in PUBLISHING.items():
                if needle in block:
                    self.assertIn(name, allowed, f"`{needle}` found in job {name}")
        # The attestations and the signature check of a dry run stay in the workflow run: attest steps are guarded.
        block = jobs()["sign-and-attest"]
        steps = re.split(r"(?m)^      - ", block)
        attest = [s for s in steps if "uses: actions/attest@" in s]
        self.assertGreaterEqual(len(attest), 2, "provenance and SBOM attestations are expected")
        for step in attest:
            self.assertIn(GUARD_EVENT, step, "an attest step must run only for a pushed tag")
            self.assertIn(GUARD_TAG, step, "an attest step must run only for a pushed tag")

    def test_only_the_known_jobs_have_write_permissions(self) -> None:
        for name, block in jobs().items():
            found = set(re.findall(r"(?m)^      ([a-z-]+: write)\b", block))
            self.assertLessEqual(found, ALLOWED_WRITES.get(name, set()), f"job {name} has unexpected write permissions")
        for name, writes in ALLOWED_WRITES.items():
            found = set(re.findall(r"(?m)^      ([a-z-]+: write)\b", jobs()[name]))
            self.assertEqual(found, writes, f"job {name}: permissions changed")

    def test_the_signing_job_does_not_publish_or_write_contents(self) -> None:
        block = jobs()["sign-and-attest"]
        self.assertNotIn("contents: write", block)
        self.assertIn("actions/upload-artifact@", block, "a dry run keeps its signed output as workflow artifacts")

    def test_every_action_is_pinned_to_a_commit_and_tools_to_exact_versions(self) -> None:
        for line in text().splitlines():
            match = re.search(r"uses: ([\w./-]+)@(\S+)", line)
            if match:
                self.assertRegex(match.group(2), r"^[0-9a-f]{40}$", f"unpinned action: {line.strip()}")
        body = text()
        self.assertRegex(body, r"cosign-release: v\d+\.\d+\.\d+\b")
        self.assertIn("cargo-cyclonedx@0.5.9", body)
        self.assertIn("requirements-sbom.txt", body)

    def test_the_version_check_does_not_block_a_dry_run(self) -> None:
        block = jobs()["check-version"]
        self.assertIn("workflow_dispatch", block + text().split("\njobs:\n", 1)[0])
        self.assertNotIn("\n    if:", block, "skipping check-version would skip every job that needs it")


if __name__ == "__main__":
    unittest.main()
