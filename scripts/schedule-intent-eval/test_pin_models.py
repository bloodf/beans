"""pin_models.py against a mock Hugging Face: the revision API, LFS hashes, and file downloads,
served from this process. No network is touched.

    python3 -I -B scripts/schedule-intent-eval/test_pin_models.py -v

The ONNX tensor checks run when `onnx` and `onnxruntime` are installed, on the tiny fixtures in
crates/cli/src/embeddings/fixtures; they are skipped otherwise.
"""
import contextlib
import hashlib
import http.server
import io
import json
import pathlib
import runpy
import sys
import tempfile
import threading
import unittest
import unittest.mock

HERE = pathlib.Path(__file__).resolve().parent
PIN = runpy.run_path(str(HERE / "pin_models.py"), run_name="pin_models_tests")
# `run_path` returns a copy: the functions read their globals from this one.
PIN_GLOBALS = PIN["main"].__globals__
FIXTURES = HERE.parents[1] / "crates" / "cli" / "src" / "embeddings" / "fixtures"
COMMIT = "c0ffee" * 6 + "c0ff"


class MockHub:
    """`/api/models/<repo>/revision/<rev>?blobs=true` and `/<repo>/resolve/<rev>/<path>`, from
    `files` ({repo: {path: bytes}}). `lfs` overrides the SHA-256 the API lists for a path (None:
    not listed, as for a small file kept outside LFS); `missing` "<repo>/<path>"s answer 404.
    Every request is logged with its Authorization header."""

    def __init__(self, files, lfs=None, missing=()):
        self.files, self.lfs, self.missing, self.requests, self.auth = files, lfs or {}, set(missing), [], []
        hub = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_GET(self):
                hub.requests.append(self.path)
                hub.auth.append(self.headers.get("Authorization"))
                path = self.path.split("?")[0]
                if path.startswith("/api/models/"):
                    repo = "/".join(path.split("/")[3:5])
                    if repo not in hub.files:
                        return self.reply(404, b"")
                    siblings = [
                        {"rfilename": name, **({"lfs": {"sha256": sha}} if (sha := hub.lfs.get(name, hashlib.sha256(data).hexdigest())) else {})}
                        for name, data in hub.files[repo].items()
                    ]
                    return self.reply(200, json.dumps({"sha": COMMIT, "siblings": siblings}).encode())
                parts = path.strip("/").split("/")
                repo, name = "/".join(parts[:2]), "/".join(parts[4:])
                data = hub.files.get(repo, {}).get(name)
                if data is None or f"{repo}/{name}" in hub.missing or parts[3] != COMMIT:
                    return self.reply(404, b"")
                self.reply(200, data)

            def reply(self, status, body):
                self.send_response(status)
                self.send_header("content-length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.endpoint = f"http://127.0.0.1:{self.server.server_address[1]}"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def close(self):
        self.server.shutdown()
        self.server.server_close()


def static_model(model_id, repo, weights=b"weights", tokenizer=b"{}"):
    return (
        {
            "id": model_id, "name": model_id, "engine": "static", "languages": "English", "license": "MIT",
            "repo": repo, "revision": "main",
            "files": {"weights": {"path": "table.safetensors", "bytes": None, "sha256": None},
                      "tokenizer": {"path": "tokenizer.json", "bytes": None, "sha256": None}},
            "approx_bytes": 100, "static": {"tensor": "embedding.weight", "dimensions": 4, "add_special_tokens": False},
            "other_sources": [], "measured": None,
        },
        {"table.safetensors": weights, "tokenizer.json": tokenizer},
    )


class PinTests(unittest.TestCase):
    def run_pin(self, models, files, *args, lfs=None, missing=(), slash=""):
        """Runs the script on `models` against a mock hub serving `files`: (exit code, the
        catalog after, what it printed, the hub)."""
        hub = MockHub(files, lfs, missing)
        self.addCleanup(hub.close)
        with tempfile.TemporaryDirectory() as scratch:
            catalog = pathlib.Path(scratch) / "catalog.json"
            catalog.write_text(json.dumps({"version": 1, "note": "test", "models": models}))
            out = io.StringIO()
            argv = ["pin_models.py", "--endpoint", hub.endpoint + slash, "--catalog", str(catalog), *args]
            with contextlib.redirect_stdout(out), unittest.mock.patch.object(sys, "argv", argv), self.assertRaises(SystemExit) as exit:
                PIN["main"]()
            return exit.exception.code, json.loads(catalog.read_text()), out.getvalue(), hub

    def test_an_unpinned_model_is_pinned_to_its_commit_and_exact_files(self):
        model, files = static_model("fixture", "owner/fixture")
        code, catalog, out, hub = self.run_pin([model], {"owner/fixture": files})
        self.assertEqual(code, 0, out)
        pinned = catalog["models"][0]
        self.assertEqual(pinned["revision"], COMMIT)
        self.assertEqual(pinned["files"]["weights"], {"path": "table.safetensors", "bytes": 7, "sha256": hashlib.sha256(b"weights").hexdigest()})
        self.assertEqual(pinned["files"]["tokenizer"]["bytes"], 2)
        self.assertNotIn("approx_bytes", pinned)
        self.assertIn(COMMIT, pinned["pinned_from"])
        self.assertTrue(all(COMMIT in path for path in hub.requests if "/resolve/" in path), "files are read at the commit")

    def test_a_file_that_is_not_what_the_hub_lists_pins_nothing(self):
        model, files = static_model("fixture", "owner/fixture")
        code, catalog, out, _ = self.run_pin([model], {"owner/fixture": files}, lfs={"tokenizer.json": "0" * 64})
        self.assertEqual(code, 1)
        self.assertIn("not pinned", out)
        self.assertEqual(catalog["models"][0]["files"]["weights"]["sha256"], None, "no file of the model is pinned")
        self.assertEqual(catalog["models"][0]["revision"], "main")

    def test_one_model_failing_leaves_the_others_pinned(self):
        good, good_files = static_model("good", "owner/good")
        bad, bad_files = static_model("bad", "owner/bad")
        code, catalog, out, _ = self.run_pin([bad, good], {"owner/good": good_files, "owner/bad": bad_files}, missing={"owner/bad/tokenizer.json"})
        self.assertEqual(code, 1, "a failure is reported")
        by_id = {m["id"]: m for m in catalog["models"]}
        self.assertEqual(by_id["good"]["revision"], COMMIT, "a later model is still pinned")
        self.assertIsNotNone(by_id["good"]["files"]["tokenizer"]["sha256"])
        self.assertEqual(by_id["bad"]["revision"], "main")
        self.assertIsNone(by_id["bad"]["files"]["weights"]["sha256"], "its weights read cleanly, but are not pinned alone")
        self.assertIn("not pinned: HTTP Error 404", out)

    def test_a_repository_the_hub_does_not_have_pins_nothing(self):
        model, _ = static_model("gone", "owner/gone")
        code, catalog, out, hub = self.run_pin([model], {})
        self.assertEqual(code, 1)
        self.assertEqual(catalog["models"][0]["revision"], "main")
        self.assertFalse(any("/resolve/" in path for path in hub.requests), "no file is fetched without a commit")

    def test_a_file_outside_lfs_is_pinned_by_its_own_hash(self):
        model, files = static_model("fixture", "owner/fixture")
        code, catalog, out, _ = self.run_pin([model], {"owner/fixture": files}, lfs={"tokenizer.json": None})
        self.assertEqual(code, 0, out)
        self.assertEqual(catalog["models"][0]["files"]["tokenizer"]["sha256"], hashlib.sha256(b"{}").hexdigest())

    def test_an_empty_file_is_refused(self):
        model, files = static_model("fixture", "owner/fixture", weights=b"")
        code, catalog, out, _ = self.run_pin([model], {"owner/fixture": files})
        self.assertEqual(code, 1)
        self.assertIn("empty", out)
        self.assertIsNone(catalog["models"][0]["files"]["weights"]["sha256"])

    def test_a_file_that_changed_upstream_is_repinned_and_reported(self):
        model, files = static_model("fixture", "owner/fixture")
        model["files"]["weights"].update(bytes=3, sha256="1" * 64)
        model["files"]["tokenizer"].update(bytes=2, sha256=hashlib.sha256(b"{}").hexdigest())
        code, catalog, out, _ = self.run_pin([model], {"owner/fixture": files}, "--model", "fixture")
        self.assertEqual(code, 0, out)
        self.assertIn("weights changed from 3 bytes " + "1" * 64, out)
        self.assertEqual(catalog["models"][0]["files"]["weights"]["sha256"], hashlib.sha256(b"weights").hexdigest())

    def test_a_token_is_sent_only_when_set(self):
        model, files = static_model("fixture", "owner/fixture")
        with unittest.mock.patch.dict("os.environ", {"HF_TOKEN": "secret"}):
            _, _, _, hub = self.run_pin([model], {"owner/fixture": files})
        self.assertEqual(set(hub.auth), {"Bearer secret"})
        with unittest.mock.patch.dict("os.environ", {}, clear=True):
            _, _, _, hub = self.run_pin([model], {"owner/fixture": files})
        self.assertEqual(set(hub.auth), {None})

    def test_an_endpoint_with_a_trailing_slash_works(self):
        model, files = static_model("fixture", "owner/fixture")
        code, _, out, hub = self.run_pin([model], {"owner/fixture": files}, slash="/")
        self.assertEqual(code, 0, out)
        self.assertTrue(hub.requests and all("//" not in path for path in hub.requests), hub.requests)

    def test_offline_pins_from_local_files_without_the_network(self):
        model, files = static_model("fixture", "owner/fixture")
        with tempfile.TemporaryDirectory() as scratch:
            for name, data in files.items():
                path = pathlib.Path(scratch) / "owner" / "fixture" / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
            code, catalog, out, hub = self.run_pin([model], {}, "--offline", scratch)
        self.assertEqual(code, 0, out)
        self.assertEqual(hub.requests, [], "offline touches no hub")
        pinned = catalog["models"][0]
        self.assertEqual((pinned["revision"], pinned["pinned_from"]), ("main", "local files"))
        self.assertEqual(pinned["files"]["weights"]["bytes"], 7)

    def test_a_model_over_the_cap_is_refused(self):
        model, files = static_model("huge", "owner/huge")
        # Each file is under the cap, together they are over it.
        with unittest.mock.patch.dict(PIN_GLOBALS, {"MAX_MODEL_BYTES": 8}):
            code, catalog, out, _ = self.run_pin([model], {"owner/huge": files})
        self.assertEqual(code, 1)
        self.assertIn("more than", out)
        self.assertIsNone(catalog["models"][0]["files"]["weights"]["sha256"])

    def test_pinned_models_are_left_alone_unless_asked(self):
        model, files = static_model("fixture", "owner/fixture")
        for role, name in (("weights", "table.safetensors"), ("tokenizer", "tokenizer.json")):
            model["files"][role].update(bytes=len(files[name]), sha256=hashlib.sha256(files[name]).hexdigest())
        code, _, out, hub = self.run_pin([model], {"owner/fixture": files})
        self.assertEqual((code, hub.requests), (0, []), "nothing to pin, nothing fetched")
        code, _, _, hub = self.run_pin([model], {"owner/fixture": files}, "--all")
        self.assertEqual(code, 0)
        self.assertTrue(hub.requests, "--all reads pinned models again")
        code, _, _, _ = self.run_pin([model], {"owner/fixture": files}, "--model", "nobody")
        self.assertNotEqual(code, 0, "an unknown id is an error")

    def test_an_image_model_pins_both_towers(self):
        self.assertEqual([role for role, _ in PIN["files_of"]({"engine": "clip", "files": {"model": 1, "image_model": 2, "tokenizer": 3}})], ["model", "image_model", "tokenizer"])
        self.assertEqual([role for role, _ in PIN["files_of"]({"engine": "onnx", "files": {"model": 1, "tokenizer": 3}})], ["model", "tokenizer"])


def onnx_available():
    try:
        import onnxruntime  # noqa: F401
        return (FIXTURES / "tiny-vision.onnx").is_file()
    except ImportError:
        return False


@unittest.skipUnless(onnx_available(), "onnxruntime is not installed")
class TensorTests(unittest.TestCase):
    def clip(self, **image):
        spec = {"input": "pixel_values", "output": "image_embeds", "size": 16, "mean": [0.5] * 3, "std": [0.5] * 3}
        spec.update(image)
        return {"onnx": {"dimensions": 4, "pooling": "pooled", "tensors": {"input_ids": "input_ids", "attention_mask": "attention_mask", "token_type_ids": None, "output": "text_embeds"}}, "image": spec}

    def test_the_fixture_towers_match_their_contracts(self):
        model = self.clip()
        PIN["check_onnx"](model, FIXTURES / "tiny-text.onnx")
        PIN["check_image_onnx"](model, FIXTURES / "tiny-vision.onnx")

    def test_a_tower_that_differs_is_refused(self):
        for change in ({"input": "pixels"}, {"output": "embeds"}, {"size": 224}):
            with self.assertRaises(ValueError, msg=str(change)):
                PIN["check_image_onnx"](self.clip(**change), FIXTURES / "tiny-vision.onnx")
        wide = self.clip()
        wide["onnx"]["dimensions"] = 512
        with self.assertRaises(ValueError):
            PIN["check_image_onnx"](wide, FIXTURES / "tiny-vision.onnx")
        with self.assertRaises(ValueError):
            PIN["check_onnx"](wide, FIXTURES / "tiny-text.onnx")


if __name__ == "__main__":
    unittest.main()
