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

    def git(self, *args):
        return subprocess.check_output(
            ["git", "-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false", *args],
            cwd=self.repository, stderr=subprocess.PIPE, text=True,
        )

    def commit(self, message):
        self.git("commit", "--quiet", "--allow-empty", "--message", message)

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


if __name__ == "__main__":
    unittest.main()
