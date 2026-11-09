import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


COMMAND = json.loads((Path(__file__).resolve().parents[1] / "web/vercel.json").read_text())["ignoreCommand"]


class IgnoreBuildTest(unittest.TestCase):
    def test_integrations_and_input_boundaries(self):
        with tempfile.TemporaryDirectory(prefix="beans-vercel-integrations-") as directory:
            root = Path(directory)
            web = root / "web"
            web.mkdir()
            (web / "vercel-ignore.sh").write_bytes((Path(__file__).resolve().parents[1] / "web/vercel-ignore.sh").read_bytes())

            def git(*args):
                return subprocess.check_output(["git", *args], cwd=root, text=True, stderr=subprocess.PIPE).strip()

            def commit():
                git("add", ".")
                git("commit", "-qm", "fixture")
                return git("rev-parse", "HEAD")

            def run(previous, branch="main", pr="", cwd=web):
                env = {**os.environ, "VERCEL_GIT_PREVIOUS_SHA": previous,
                       "VERCEL_GIT_COMMIT_REF": branch, "VERCEL_GIT_PULL_REQUEST_ID": pr}
                return subprocess.run(["sh", "-c", COMMAND], cwd=cwd, env=env, capture_output=True, text=True)

            git("init", "-qb", "main")
            git("config", "user.name", "Beans test")
            git("config", "user.email", "test@example.invalid")
            baseline = commit()
            inputs = ("web/page.txt", "desktop/src/demo.txt", "package.json", "pnpm-lock.yaml",
                      "pnpm-workspace.yaml", ".npmrc", ".nvmrc", ".node-version", ".pnpmfile.cjs",
                      ".pnpmfile.mjs", "tsconfig.json", "patches/dependency.patch",
                      "packages/beans-blobatar/avatar.txt", "crates/models/catalog.json",
                      "crates/cli/marketplace/index.json", ".vercelignore")
            for name in inputs:
                with self.subTest(input=name):
                    previous = git("rev-parse", "HEAD")
                    path = root / name
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text("new input")
                    commit()
                    self.assertEqual(run(previous).returncode, 1)
            for branch in ("feature/site", "release/x/y", ".hidden", "main/child", "Main", ""):
                with self.subTest(branch=branch):
                    self.assertEqual(run(baseline, branch=branch).returncode, 0)
            self.assertEqual(run(baseline, pr="123").returncode, 0)
            previous = git("rev-parse", "HEAD")
            (web / "page.txt").unlink()
            commit()
            self.assertEqual(run(previous).returncode, 1)
            previous = git("rev-parse", "HEAD")
            git("mv", "desktop/src/demo.txt", "outside.txt")
            commit()
            self.assertEqual(run(previous).returncode, 1)
            previous = git("rev-parse", "HEAD")
            git("mv", "outside.txt", "web/renamed.txt")
            commit()
            self.assertEqual(run(previous).returncode, 1)
            for merge in ("--no-ff", "--squash"):
                with self.subTest(merge=merge):
                    previous = git("rev-parse", "HEAD")
                    git("checkout", "-qb", "feature" + merge)
                    (web / "merged.txt").write_text(merge)
                    commit()
                    (root / "unrelated.txt").write_text(merge)
                    commit()
                    git("checkout", "-q", "main")
                    git("merge", merge, "-m", "integrate", "feature" + merge)
                    if merge == "--squash":
                        commit()
                    self.assertEqual(run(previous).returncode, 1)
            previous = git("rev-parse", "HEAD")
            (root / "Cargo.lock").write_text("Rust only")
            commit()
            self.assertEqual(run(previous).returncode, 0)
            shallow = root / "shallow"
            git("clone", "--depth=1", "--quiet", root.as_uri(), str(shallow))
            result = run(previous, cwd=shallow / "web")
            self.assertEqual(result.returncode, 0)
            self.assertIn("changes unknown", result.stdout)
            self.assertEqual(run(git("rev-parse", "HEAD"), cwd=shallow / "web").returncode, 0)
            # A corrupt candidate HEAD causes a diff error, not an authorized build.
            (shallow / ".git/HEAD").write_text("ref: refs/heads/missing\n")
            result = run(git("rev-parse", "HEAD"), cwd=shallow / "web")
            self.assertEqual(result.returncode, 0)
            self.assertIn("Git comparison failed", result.stdout)

    def test_main_only_configuration(self):
        config = json.loads((Path(__file__).resolve().parents[1] / "web/vercel.json").read_text())
        self.assertEqual(config["git"]["deploymentEnabled"], {"**": False, "main": True})

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
            env = {**os.environ, "VERCEL_GIT_COMMIT_REF": "main", "VERCEL_GIT_PULL_REQUEST_ID": "", "VERCEL_GIT_PREVIOUS_SHA": previous}
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
                env = {**os.environ, "VERCEL_GIT_COMMIT_REF": "main", "VERCEL_GIT_PULL_REQUEST_ID": "", "VERCEL_GIT_PREVIOUS_SHA": previous}
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
            self.assertEqual(ignored(""), 0)
            self.assertEqual(ignored("f" * 40), 0)
            for invalid in ("--cached", "--quiet", "HEAD", "refs/heads/main", "g" * 40, "a" * 39, "a" * 41, "a" * 63, "a" * 65, "f" * 64):
                with self.subTest(previous=invalid):
                    self.assertEqual(ignored(invalid), 0)
            missing_env = {**os.environ, "VERCEL_GIT_COMMIT_REF": "main", "VERCEL_GIT_PULL_REQUEST_ID": ""}
            missing_env.pop("VERCEL_GIT_PREVIOUS_SHA", None)
            self.assertEqual(subprocess.run(["sh", "-c", COMMAND], cwd=web, env=missing_env, capture_output=True).returncode, 0)
            blob = git("rev-parse", "HEAD:web/page.txt")
            tree = git("rev-parse", "HEAD^{tree}")
            self.assertEqual(ignored(blob), 0)
            self.assertEqual(ignored(tree), 0)
            (root / ".git").rename(root / "git-unavailable")
            try:
                self.assertEqual(ignored(initial), 0)
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
