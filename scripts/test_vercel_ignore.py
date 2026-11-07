import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


COMMAND = json.loads((Path(__file__).resolve().parents[1] / "web/vercel.json").read_text())["ignoreCommand"]


class IgnoreBuildTest(unittest.TestCase):
    def test_sha256_commit(self):
        with tempfile.TemporaryDirectory(prefix="beans-vercel-sha256-") as directory:
            root = Path(directory)
            web = root / "web"
            web.mkdir()
            (web / "vercel-ignore.sh").write_bytes((Path(__file__).resolve().parents[1] / "web/vercel-ignore.sh").read_bytes())
            subprocess.run(["git", "init", "-q", "--object-format=sha256"], cwd=root, check=True, capture_output=True)
            subprocess.run(["git", "add", "."], cwd=root, check=True, capture_output=True)
            subprocess.run(["git", "-c", "user.name=Beans test", "-c", "user.email=test@example.invalid", "commit", "-qm", "initial"], cwd=root, check=True, capture_output=True)
            previous = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
            self.assertEqual(len(previous), 64)
            env = {**os.environ, "VERCEL_GIT_PREVIOUS_SHA": previous}
            self.assertEqual(subprocess.run(["sh", "-c", COMMAND], cwd=web, env=env, capture_output=True).returncode, 0)
            (web / "page.txt").write_text("changed")
            subprocess.run(["git", "add", "."], cwd=root, check=True, capture_output=True)
            subprocess.run(["git", "-c", "user.name=Beans test", "-c", "user.email=test@example.invalid", "commit", "-qm", "changed"], cwd=root, check=True, capture_output=True)
            self.assertEqual(subprocess.run(["sh", "-c", COMMAND], cwd=web, env=env, capture_output=True).returncode, 1)

    def test_real_git_inputs(self):
        with tempfile.TemporaryDirectory(prefix="beans-vercel-ignore-") as directory:
            root = Path(directory)
            web = root / "web"
            web.mkdir()
            self.assertLessEqual(len(COMMAND), 256)
            (web / "vercel-ignore.sh").write_bytes((Path(__file__).resolve().parents[1] / "web/vercel-ignore.sh").read_bytes())

            def git(*args):
                return subprocess.run(["git", *args], cwd=root, check=True, capture_output=True, text=True).stdout.strip()

            def ignored(previous):
                env = {**os.environ, "VERCEL_GIT_PREVIOUS_SHA": previous}
                return subprocess.run(["sh", "-c", COMMAND], cwd=web, env=env, capture_output=True).returncode

            git("init", "-q")
            git("config", "user.name", "Beans test")
            git("config", "user.email", "test@example.invalid")
            (web / "page.txt").write_text("initial")
            (root / "pnpm-lock.yaml").write_text("initial")
            git("add", ".")
            git("commit", "-qm", "initial")
            initial = git("rev-parse", "HEAD")
            self.assertEqual(ignored(initial), 0)
            self.assertEqual(ignored(""), 1)
            self.assertEqual(ignored("f" * 40), 1)
            for invalid in ("--cached", "--quiet", "HEAD", "refs/heads/main", "g" * 40, "a" * 39, "a" * 41, "a" * 63, "a" * 65, "f" * 64):
                with self.subTest(previous=invalid):
                    self.assertEqual(ignored(invalid), 1)
            missing_env = dict(os.environ)
            missing_env.pop("VERCEL_GIT_PREVIOUS_SHA", None)
            self.assertEqual(subprocess.run(["sh", "-c", COMMAND], cwd=web, env=missing_env, capture_output=True).returncode, 1)
            blob = git("rev-parse", "HEAD:web/page.txt")
            tree = git("rev-parse", "HEAD^{tree}")
            self.assertEqual(ignored(blob), 1)
            self.assertEqual(ignored(tree), 1)
            (root / ".git").rename(root / "git-unavailable")
            try:
                self.assertEqual(ignored(initial), 1)
            finally:
                (root / "git-unavailable").rename(root / ".git")
            (web / "page.txt").write_text("changed")
            git("commit", "-qam", "website change")
            self.assertEqual(ignored(initial), 1)
            website = git("rev-parse", "HEAD")
            (root / "pnpm-lock.yaml").write_text("changed")
            git("commit", "-qam", "dependency change")
            self.assertEqual(ignored(website), 1)
            dependency = git("rev-parse", "HEAD")
            (root / "Cargo.lock").write_text("Rust-only change")
            git("add", ".")
            git("commit", "-qm", "unrelated Rust change")
            self.assertEqual(ignored(dependency), 0)


if __name__ == "__main__":
    unittest.main()
