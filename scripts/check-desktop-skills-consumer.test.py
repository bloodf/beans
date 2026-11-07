#!/usr/bin/env python3
"""Run with python -I scripts/check-desktop-skills-consumer.test.py."""
import base64
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import sys

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("driver", Path(__file__).with_name("check-desktop-skills-consumer.py"))
driver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(driver)

with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    subprocess.run(["git", "init", "-q", directory], check=True)
    subprocess.run(["git", "-C", directory, "config", "user.email", "test@example.invalid"], check=True)
    subprocess.run(["git", "-C", directory, "config", "user.name", "Test"], check=True)
    (root / "desktop/src").mkdir(parents=True)
    source = root / "desktop/src/runner.ts"
    source.write_text("export const result = 1;\n")
    (root / "desktop/src/icon.png").write_bytes(b"PNG_FIXTURE")
    subprocess.run(["git", "-C", directory, "add", "."], check=True)
    subprocess.run(["git", "-C", directory, "-c", "commit.gpgsign=false", "commit", "-qm", "fixture"], check=True)
    clean = driver.source_binding(root)
    assert clean["files"]["desktop/src/runner.ts"]["tracked"]
    assert not clean["files"]["desktop/src/runner.ts"]["dirty"]
    source.write_text("export const result = 2;\n")
    (root / "desktop/src/generated.ts").write_text("export {};\n")
    dirty = driver.source_binding(root)
    assert dirty["files"]["desktop/src/runner.ts"]["dirty"]
    assert not dirty["files"]["desktop/src/generated.ts"]["tracked"]
    assert clean != dirty
    with tempfile.TemporaryDirectory() as destination:
        driver.freeze_sources(root, Path(destination), dirty)
        frozen = Path(destination) / "desktop/src/runner.ts"
        assert frozen.read_bytes() == source.read_bytes()
        assert (Path(destination) / "desktop/src/icon.png").read_bytes() == b"PNG_FIXTURE"
        encoded = base64.b64encode(json.dumps({"sourcesContent": [frozen.read_text()]}).encode())
        transformed = b"transformed();\n//# sourceMappingURL=data:application/json;base64," + encoded
        driver.validate_source_map(transformed, frozen)
        for body in (b"no map", transformed.replace(encoded, base64.b64encode(b'{"sourcesContent":["stale"]}'))):
            try:
                driver.validate_source_map(body, frozen)
            except RuntimeError:
                pass
            else:
                raise AssertionError("unbound transformed source must fail closed")
        assert driver.owned_source("http://localhost:123/src/runner.ts", "http://localhost:123", Path(destination)) == frozen.resolve()
        try:
            driver.owned_source("http://localhost:123/@fs/etc/passwd", "http://localhost:123", Path(destination))
        except RuntimeError:
            pass
        else:
            raise AssertionError("external owned sources must fail closed")
        source.write_text("changed after capture")
        assert frozen.read_text() == "export const result = 2;\n"
        try:
            driver.freeze_sources(root, Path(destination), dirty)
        except RuntimeError:
            pass
        else:
            raise AssertionError("changed sources must fail closed")

# Bun stores shared workspace dependencies under the root node_modules/.bun tree.
# Runtime dependency modules are recorded by executed-byte binding, not treated as owned sources.
with tempfile.TemporaryDirectory() as destination:
    base = "http://localhost:123"
    dependency = driver.ROOT / "node_modules/.bun/vite/node_modules/vite/dist/client/env.mjs"
    assert driver.owned_source(f"{base}/@fs{dependency}", base, Path(destination)) is None
    for external in [driver.ROOT / "packages/beans-blobatar/src/index.ts", driver.DESKTOP / "src/model/store.ts"]:
        try:
            driver.owned_source(f"{base}/@fs{external}", base, Path(destination))
        except RuntimeError:
            pass
        else:
            raise AssertionError("unfrozen owned workspace sources must fail closed")

with tempfile.TemporaryDirectory() as destination:
    frozen = Path(destination).resolve()
    asset = frozen / "desktop/src/icon.png"
    asset.parent.mkdir(parents=True)
    asset.write_bytes(b"PNG_FIXTURE")
    base = "http://localhost:123"
    url = base + "/src/icon.png"
    driver.validate_owned_asset(b"PNG_FIXTURE", asset, url, base, frozen)
    driver.validate_owned_asset(b'export default "/src/icon.png"', asset, url + "?import", base, frozen)
    driver.validate_owned_asset(b'export default "/src/icon.png"\n//# sourceMappingURL=data:application/json;base64,e30=', asset, url + "?import", base, frozen)
    for body, request in [(b"stale", url), (b'export default "/src/other.png"', url + "?import"),
                          (b'export default "//src/icon.png"', url + "?import"),
                          (b'export default "/src/icon.png"; malicious()', url + "?import")]:
        try:
            driver.validate_owned_asset(body, asset, request, base, frozen)
        except RuntimeError:
            pass
        else:
            raise AssertionError("unbound assets or executable wrappers must fail closed")

assets = {"http://localhost:123/src/runner.ts": {"body": b"export const result = 2;\n", "sha256": "recorded"}}
assert driver.bind_executed(assets, [("http://localhost:123/src/runner.ts", "export const result = 2;\n")])
for executed in [[("http://localhost:123/src/runner.ts", "stale")], [("http://localhost:123/missing.ts", "unrecorded")], []]:
    try:
        driver.bind_executed(assets, executed)
    except RuntimeError:
        pass
    else:
        raise AssertionError("unbound execution must fail closed")
print("PASS: clean/dirty/untracked snapshots, source races, executed-byte mismatch and missing execution")
