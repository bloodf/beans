import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


COMMAND = json.loads((Path(__file__).resolve().parents[1] / "web/vercel.json").read_text())["ignoreCommand"]


class IgnoreBuildTest(unittest.TestCase):
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
