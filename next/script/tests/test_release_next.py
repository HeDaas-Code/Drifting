import importlib.util
import tempfile
import json
import os
import subprocess
from unittest.mock import patch
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("release_next", Path(__file__).parents[1] / "release-next.py")
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def test_first_alpha_and_published_tags_and_reservations_allocate_once(self):
        self.assertEqual(release.next_version([], {}, "a"), ("0.2.0-alpha.1", None))
        releases = [{"tag_name": "0.2.0-alpha.999"}, {"tag_name": "v0.1.1"}, {"tag_name": "next-v0.2.0-alpha.9"}]
        reservations = {"0.2.0-alpha.10": ("a", "commit-a")}
        self.assertEqual(release.next_version(releases, reservations, "a"), ("0.2.0-alpha.10", "commit-a"))
        self.assertEqual(release.next_version(releases, reservations, "b"), ("0.2.0-alpha.11", None))

    def test_only_application_version_packages_and_their_lock_entries_change(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for name in ["drifting", "dr-platform", "other"]:
                folder = root / "next" / name
                folder.mkdir(parents=True)
                version = "0.0.0" if name == "other" else "0.2.0-dev"
                (folder / "Cargo.toml").write_text(f'[package]\nname = "{name}"\nversion = "{version}"\n')
            lock = root / "next" / "Cargo.lock"
            lock.write_text('version = 4\n\n[[package]]\nname = "drifting"\nversion = "0.2.0-dev"\n\n[[package]]\nname = "dr-platform"\nversion = "0.2.0-dev"\n\n[[package]]\nname = "other"\nversion = "0.0.0"\n')
            changed = release.set_versions(root, "0.2.0-alpha.1")
            self.assertEqual(len(changed), 3)
            self.assertEqual(lock.read_text().count('version = "0.2.0-alpha.1"'), 2)
            self.assertIn('version = "0.0.0"', lock.read_text())

    def test_notes_have_feature_groups_shas_and_exact_first_range(self):
        notes = release.release_notes("0.2.0-alpha.1", "a" * 40, "b" * 40, release.BASELINE, [
            ("c" * 40, "Support configurable model gateway protocols"),
            ("d" * 40, "Writing Pomodoro timer"),
        ], "example/Drifting")
        self.assertIn("## 模型兼容连接", notes)
        self.assertIn("## 番茄钟", notes)
        self.assertIn("ccccccccc", notes)
        self.assertIn(f"{release.BASELINE}..{'a' * 40}", notes)
        self.assertIn("drifting-next-source:" + "a" * 40, notes)

    def test_prepare_reserves_one_remote_commit_and_reuses_it(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            root = directory / "source"
            remote = directory / "remote.git"
            output = directory / "metadata"
            subprocess.run(["git", "init", "--bare", str(remote)], check=True, capture_output=True)
            root.mkdir()
            subprocess.run(["git", "init", str(root)], check=True, capture_output=True)
            package = root / "next" / "app"
            package.mkdir(parents=True)
            (package / "Cargo.toml").write_text('[package]\nname = "drifting"\nversion = "0.2.0-dev"\n')
            (root / "next" / "Cargo.lock").write_text('version = 4\n\n[[package]]\nname = "drifting"\nversion = "0.2.0-dev"\n')
            actual_run = release.run
            old_cwd = Path.cwd()
            try:
                os.chdir(root)
                actual_run("git", "add", ".")
                actual_run("git", "-c", "user.name=Test", "-c", "user.email=test@example.test", "commit", "-m", "Synthetic baseline")
                source = actual_run("git", "rev-parse", "HEAD")
                actual_run("git", "remote", "add", "origin", str(remote))
                releases = []
                def command(*args):
                    return json.dumps([releases]) if args[0] == "gh" else actual_run(*args)
                with patch.object(release, "BASELINE", source), patch.object(release, "run", side_effect=command), patch.dict(os.environ, {
                    "GITHUB_REPOSITORY": "example/Drifting",
                    "GITHUB_OUTPUT": str(directory / "outputs"),
                }):
                    release.prepare(source, output)
                    first = json.loads((output / "release.json").read_text())
                    self.assertEqual(first["version"], "0.2.0-alpha.1")
                    self.assertEqual(actual_run("git", "rev-parse", first["commit"] + "^"), source)
                    self.assertIn('version = "0.2.0-alpha.1"', actual_run("git", "show", first["commit"] + ":next/Cargo.lock"))
                    actual_run("git", "checkout", "--detach", source)
                    release.prepare(source, output)
                    self.assertEqual(first, json.loads((output / "release.json").read_text()))
                    releases.append({"tag_name": first["tag"], "draft": False})
                    release.prepare(source, output)
                    self.assertEqual(json.loads((output / "release.json").read_text())["published"], "true")
                    refs = actual_run("git", "ls-remote", "--heads", "origin").splitlines()
                    self.assertEqual(len(refs), 1)
            finally:
                os.chdir(old_cwd)


if __name__ == "__main__":
    unittest.main()
