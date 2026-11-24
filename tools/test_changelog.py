#!/usr/bin/env python3
"""Exercise cliff.toml against real temporary Git histories using git-cliff."""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
GIT_CLIFF = os.environ.get("GIT_CLIFF", "git-cliff")


class ChangelogTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="tessipher-changelog-")
        self.addCleanup(self.temp.cleanup)
        self.repository = Path(self.temp.name)
        self.git("init", "--quiet", "--initial-branch=main")
        self.git("config", "user.name", "Changelog test")
        self.git("config", "user.email", "test@example.invalid")

    def git(self, *args, timestamp=None):
        env = None if timestamp is None else dict(
            os.environ, GIT_AUTHOR_DATE=timestamp, GIT_COMMITTER_DATE=timestamp,
        )
        return subprocess.check_output(
            ["git", "-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false", *args],
            cwd=self.repository, env=env, stderr=subprocess.PIPE, text=True,
        )

    def commit(self, message, *, timestamp=None):
        self.git("commit", "--quiet", "--allow-empty", "--message", message,
                 timestamp=timestamp)

    def generate(self, *args):
        return subprocess.check_output(
            [GIT_CLIFF, "--config", str(ROOT / "cliff.toml"), "--offline",
             "--repository", str(self.repository), "--use-branch-tags", *args],
            cwd=self.repository, stderr=subprocess.PIPE, text=True,
        )

    def test_first_tag_contains_initial_history(self):
        self.commit("feat(qr): decode matrices")
        self.commit("chore(release): prepare v0.1.0")
        self.git("tag", "v0.1.0")
        changelog = self.generate()
        self.assertIn("## v0.1.0", changelog)
        self.assertIn("**qr:** Decode matrices", changelog)
        self.assertNotIn("Prepare v0.1.0", changelog)

    def test_current_notes_exclude_previous_release(self):
        self.commit("feat: initial decoder")
        self.git("tag", "v0.1.0")
        self.commit("fix(ffi): reject overlapping buffers")
        self.commit("docs: clarify ownership")
        self.git("tag", "v0.1.1")
        notes = self.generate("--current", "--strip", "header")
        self.assertNotIn("# Changelog", notes)
        self.assertNotIn("Initial decoder", notes)
        self.assertIn("Reject overlapping buffers", notes)
        self.assertIn("Clarify ownership", notes)
        full = self.generate()
        self.assertIn("## v0.1.0", full)
        self.assertIn("Initial decoder", full)

    def test_breaking_changes_survive_internal_type_filter(self):
        self.commit("chore(api)!: change buffer layout")
        self.commit("refactor(parser): change return type\n\n"
                    "BREAKING CHANGE: Query memory before decoding.\n"
                    "Allocate the returned capacity.")
        self.commit("ci: internal workflow cleanup")
        self.commit("Merge pull request #1")
        changelog = self.generate()
        self.assertIn("### Breaking changes", changelog)
        self.assertIn("Change buffer layout", changelog)
        self.assertIn("Query memory before decoding.", changelog)
        self.assertIn("Allocate the returned capacity.", changelog)
        self.assertNotIn("workflow cleanup", changelog)
        self.assertNotIn("Merge pull request", changelog)

    def test_prereleases_are_distinct_sections_and_current_is_tag_delta(self):
        self.commit("feat: initial decoder")
        self.git("tag", "v0.1.0")
        self.commit("feat: decode micro symbols")
        self.git("tag", "v0.2.0-rc.1")
        self.commit("fix: correct micro orientation")
        self.git("tag", "v0.2.0-rc.2")
        notes = self.generate("--current", "--strip", "header")
        self.assertIn("Correct micro orientation", notes)
        self.assertNotIn("Decode micro symbols", notes)
        self.commit("chore(release): prepare stable release")
        self.git("tag", "v0.2.0")
        full = self.generate()
        for tag in ("v0.1.0", "v0.2.0-rc.1", "v0.2.0-rc.2", "v0.2.0"):
            self.assertIn(f"## {tag}\n", full)
        self.assertNotIn("Prepare stable release", full)

    def test_unrelated_branch_tag_does_not_enter_history(self):
        self.commit("feat: main decoder")
        self.git("tag", "v0.1.0")
        self.git("checkout", "--quiet", "--orphan", "unrelated")
        self.commit("feat: unrelated decoder")
        self.git("tag", "v99.0.0")
        self.git("checkout", "--quiet", "main")
        self.commit("fix: main correction")
        self.git("tag", "v0.1.1")
        full = self.generate()
        self.assertIn("Main correction", full)
        self.assertNotIn("v99.0.0", full)
        self.assertNotIn("Unrelated decoder", full)

    def test_detached_tag_checkout_matches_actions(self):
        self.commit("feat: initial decoder")
        self.git("tag", "v0.1.0")
        self.commit("fix: update image limits")
        self.git("tag", "v0.1.1")
        self.git("checkout", "--quiet", "--detach", "v0.1.1")
        notes = self.generate("--current", "--strip", "header")
        self.assertIn("## v0.1.1", notes)
        self.assertIn("Update image limits", notes)
        self.assertNotIn("Initial decoder", notes)

    def test_current_notes_exclude_release_tag_merged_from_side_branch(self):
        self.commit("feat: initial decoder", timestamp="2026-01-01T00:00:00Z")
        self.git("tag", "v0.1.0")
        self.commit("feat: add detection modes", timestamp="2026-01-02T00:00:00Z")
        self.git("switch", "--quiet", "-c", "previous-release")
        self.commit("chore(release): prepare v0.2.0", timestamp="2026-01-03T00:00:00Z")
        self.git("tag", "-a", "v0.2.0", "-m", "Tessipher 0.2.0",
                 timestamp="2026-01-03T00:01:00Z")
        self.git("switch", "--quiet", "main")
        self.commit("fix: improve image decoding", timestamp="2026-01-04T00:00:00Z")
        self.git("merge", "--quiet", "--no-ff", "-m", "Merge previous release",
                 "previous-release", timestamp="2026-01-05T00:00:00Z")
        self.commit("chore(release): prepare v0.3.0", timestamp="2026-01-06T00:00:00Z")
        # Match CI fetching the older tags before discovering the new release tag.
        self.git("pack-refs", "--all")
        self.git("tag", "-a", "v0.3.0", "-m", "Tessipher 0.3.0",
                 timestamp="2026-01-06T00:01:00Z")
        self.git("switch", "--quiet", "--detach", "v0.3.0")

        notes = self.generate("--current", "--strip", "header")
        self.assertIn("## v0.3.0", notes)
        self.assertIn("Improve image decoding", notes)
        self.assertNotIn("Add detection modes", notes)

        full = self.generate()
        current, previous = full.split("## v0.2.0\n", maxsplit=1)
        self.assertIn("Improve image decoding", current)
        self.assertNotIn("Improve image decoding", previous)
        self.assertIn("Add detection modes", previous)


if __name__ == "__main__":
    unittest.main()
