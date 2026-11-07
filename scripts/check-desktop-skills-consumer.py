#!/usr/bin/env python3
"""Run mounted skills checks against frozen local sources and bind executed Vite bytes.

Requires existing desktop dependencies, generated src/mygo.ts and Python Playwright/Chromium.
No installation is performed. Receipts distinguish source hashes from transformed script hashes.
"""
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
DESKTOP = ROOT / "desktop"
PAGE = "/src/ui/sheets/skillsMountedCheck.html"
TIMEOUT_SECONDS = 120
ASSET_SUFFIXES = {".svg", ".png", ".jpg", ".jpeg", ".gif", ".webp", ".woff", ".woff2", ".ico"}
EXCLUDED = {"node_modules", ".git", ".mygo", "build", "dist", "resources", "__pycache__"}


def git(root, *args):
    return subprocess.run(["git", *args], cwd=root, capture_output=True, check=True).stdout


def source_binding(root=ROOT):
    """Capture all owned frontend inputs, including ignored generated and dirty files."""
    root = Path(root)
    tracked = set(git(root, "ls-files", "-z").decode().split("\0"))
    files = {}
    for directory in (root / "desktop", root / "packages"):
        if not directory.exists():
            continue
        for current, directories, names in os.walk(directory, followlinks=False):
            directories[:] = sorted(name for name in directories if name not in EXCLUDED and not name.startswith("."))
            for name in directories:
                if (Path(current) / name).is_symlink():
                    raise RuntimeError(f"owned source directory symlink refused: {name}")
            for name in sorted(names):
                if name.startswith(".") or Path(name).suffix not in ({".ts", ".tsx", ".js", ".mjs", ".cjs", ".html", ".css", ".json"} | ASSET_SUFFIXES):
                    continue
                if Path(name).suffix == ".json" and name not in {"package.json", "tsconfig.json"}:
                    continue
                path = Path(current) / name
                if path.is_symlink():
                    raise RuntimeError(f"owned source symlink refused: {path.relative_to(root)}")
                label = path.relative_to(root).as_posix()
                data = path.read_bytes()
                head = git(root, "show", f"HEAD:{label}") if label in tracked and subprocess.run(
                    ["git", "cat-file", "-e", f"HEAD:{label}"], cwd=root, capture_output=True).returncode == 0 else None
                files[label] = {"sha256": hashlib.sha256(data).hexdigest(), "tracked": label in tracked,
                                "dirty": head != data}
    for path in (Path(__file__), Path(__file__).with_name("check-desktop-skills-consumer.test.py")):
        if root == ROOT and path.exists():
            label = path.relative_to(root).as_posix()
            data = path.read_bytes()
            files[label] = {"sha256": hashlib.sha256(data).hexdigest(), "tracked": label in tracked,
                            "dirty": subprocess.run(["git", "show", f"HEAD:{label}"], cwd=root,
                                                    capture_output=True).stdout != data}
    return {"commit": git(root, "rev-parse", "HEAD").decode().strip(),
            "status": git(root, "status", "--porcelain=v1", "--untracked-files=all").decode(),
            "files": files}


def freeze_sources(root, destination, binding):
    for label, entry in binding["files"].items():
        data = (Path(root) / label).read_bytes()
        if hashlib.sha256(data).hexdigest() != entry["sha256"]:
            raise RuntimeError(f"source changed during capture: {label}")
        target = Path(destination) / label
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)


def bind_executed(assets, executed):
    if not executed:
        raise RuntimeError("no executed scripts recorded")
    receipt = []
    for url, source in executed:
        if url not in assets or assets[url]["body"] != source.encode("utf-8"):
            raise RuntimeError(f"executed bytes do not match intercepted response: {url}")
        receipt.append({"url": url, "executed_sha256": hashlib.sha256(source.encode("utf-8")).hexdigest()})
    return receipt


def owned_source(url, base, frozen):
    parsed = urllib.parse.urlsplit(url)
    path = urllib.parse.unquote(parsed.path)
    if path.startswith("/@fs/"):
        candidate = Path(path[4:]).resolve()
    elif path.startswith(("/@vite/", "/node_modules/", "/@id/")):
        return None
    else:
        candidate = (frozen / "desktop" / path.lstrip("/")).resolve()
    if any(candidate.is_relative_to(directory.resolve()) for directory in
           (DESKTOP / "node_modules", ROOT / "node_modules")):
        return None
    if not candidate.is_relative_to(frozen.resolve()):
        raise RuntimeError(f"owned module outside frozen tree: {url}")
    return candidate


def validate_source_map(body, source):
    matches = re.findall(rb"//# sourceMappingURL=data:application/json[^,]*;base64,([^\s]+)", body)
    if len(matches) != 1:
        raise RuntimeError(f"missing unique inline source map: {source}")
    mapping = json.loads(base64.b64decode(matches[0], validate=True))
    if source.read_bytes().decode("utf-8") not in mapping.get("sourcesContent", []):
        raise RuntimeError(f"transformed source map does not bind frozen bytes: {source}")


def validate_owned_asset(body, source, url, base, frozen):
    query = urllib.parse.parse_qs(urllib.parse.urlsplit(url).query, keep_blank_values=True)
    if "import" in query or "url" in query:
        # Vite asset imports contain only a URL export, never arbitrary executable code.
        wrapper = re.sub(rb"\n//# sourceMappingURL=data:application/json[^,\n]*;base64,[A-Za-z0-9+/=]+\s*$", b"", body)
        match = re.fullmatch(rb'export default ("(?:[^"\\]|\\.)*")\s*;?\s*', wrapper)
        if not match:
            raise RuntimeError(f"invalid asset URL module: {source}")
        target = json.loads(match[1])
        if not target.startswith("/") or target.startswith("//") or owned_source(base + target, base, frozen) != source:
            raise RuntimeError(f"asset URL does not bind frozen path: {source}")
    elif body != source.read_bytes():
        raise RuntimeError(f"served asset does not bind frozen bytes: {source}")


def free_port():
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


def wait_for(url, process):
    deadline = time.monotonic() + TIMEOUT_SECONDS
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"vite exited with {process.returncode} before serving checks")
        try:
            with urllib.request.urlopen(url, timeout=2):
                return
        except (urllib.error.URLError, OSError):
            time.sleep(0.2)
    raise RuntimeError("vite did not serve checks within 120s")


def run_browser(base, frozen):
    from playwright.sync_api import sync_playwright
    assets, scripts, errors, console = {}, [], [], []
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch()
        try:
            context = browser.new_context(service_workers="block")
            def intercept(route):
                url = route.request.url
                if not url.startswith(base + "/"):
                    errors.append(f"nonlocal request refused: {url}")
                    route.abort()
                    return
                try:
                    if url == base + PAGE:
                        source = frozen / "desktop" / PAGE.lstrip("/")
                        body = source.read_bytes()
                        assets[url] = {"body": body, "sha256": hashlib.sha256(body).hexdigest(),
                                       "source": source.relative_to(frozen).as_posix()}
                        route.fulfill(status=200, content_type="text/html; charset=utf-8", body=body)
                        return
                    response = route.fetch(max_redirects=0)
                    body = response.body()
                    if response.status != 200:
                        raise RuntimeError(f"asset status {response.status}: {url}")
                    if url in assets and assets[url]["body"] != body:
                        raise RuntimeError(f"asset changed during execution: {url}")
                    source = owned_source(url, base, frozen)
                    if source and source.suffix in ASSET_SUFFIXES:
                        validate_owned_asset(body, source, url, base, frozen)
                    elif source and source.suffix != ".html":
                        validate_source_map(body, source)
                    assets[url] = {"body": body, "sha256": hashlib.sha256(body).hexdigest(),
                                   "source": source.relative_to(frozen).as_posix() if source else None}
                    route.fulfill(response=response, body=body)
                except Exception as error:
                    errors.append(str(error))
                    route.abort()
            context.route("**/*", intercept)
            page = context.new_page()
            page.on("console", lambda message: console.append(f"{message.type}: {message.text}"))
            page.on("pageerror", lambda error: errors.append(str(error)))
            session = context.new_cdp_session(page)
            session.send("Debugger.enable")
            session.send("Profiler.enable")
            session.send("Profiler.startPreciseCoverage", {"callCount": True, "detailed": True})
            session.send("Network.setCacheDisabled", {"cacheDisabled": True})
            session.on("Debugger.scriptParsed", lambda event: scripts.append(event) if event.get("url") else None)
            try:
                page.goto(base + PAGE, wait_until="load")
                if errors:
                    raise RuntimeError("asset loading failed")
                page.wait_for_function("window.__result !== undefined || window.__error !== undefined", timeout=60000)
            except Exception as error:
                return {"result": None, "error": "; ".join([*errors, str(error)]), "console": console}
            result, error = page.evaluate("[window.__result ?? null, window.__error ?? null]")
            executed = [(event["url"], session.send("Debugger.getScriptSource", {"scriptId": event["scriptId"]})["scriptSource"])
                        for event in scripts]
            execution = bind_executed(assets, executed)
            coverage = session.send("Profiler.takePreciseCoverage")["result"]
            active = {entry["url"] for entry in coverage if any(
                span["count"] > 0 for function in entry["functions"] for span in function["ranges"])}
            required = {"desktop/src/ui/sheets/skillsMountedCheck.ts", "desktop/src/ui/sheets/skillDiscovery.browser.tsx",
                        "desktop/src/ui/sheets/skillLibrary.browser.tsx", "desktop/src/ui/sheets/skillDiscovery.tsx",
                        "desktop/src/model/reactive.ts", "desktop/src/model/store.ts", "desktop/src/l10n/zh.ts"}
            seen = {assets[url]["source"] for url, _ in executed if url in active}
            if required - seen or base + PAGE not in assets:
                errors.append(f"required mounted inputs did not execute: {sorted(required - seen)}")
            missing = [name for name in ("discovery", "library", "keepBoth", "compare") if not (result or {}).get(name)]
            if missing:
                errors.append(f"no result reported for: {', '.join(missing)}")
            if error:
                errors.append(error)
            return {"result": result, "error": "; ".join(errors) or None, "console": console,
                    "execution": [{**entry, "executed": entry["url"] in active} for entry in execution],
                    "assets": {url: {key: value for key, value in asset.items() if key != "body"}
                                                       for url, asset in assets.items()}}
        finally:
            browser.close()


def main():
    receipt = {"result": None, "error": None}
    try:
        if not (DESKTOP / "src/mygo.ts").is_file() or not (DESKTOP / "node_modules").is_dir():
            raise RuntimeError("existing desktop/node_modules and desktop/src/mygo.ts required; no installation performed")
        binding = source_binding()
        receipt["source"] = binding
        with tempfile.TemporaryDirectory(prefix="beans-skills-binding-") as directory:
            frozen = Path(directory).resolve()
            freeze_sources(ROOT, frozen, binding)
            (frozen / "desktop/node_modules").symlink_to(DESKTOP / "node_modules", target_is_directory=True)
            port = free_port()
            base = f"http://localhost:{port}"
            vite = subprocess.Popen(["bun", str(DESKTOP / "node_modules/vite/bin/vite.js"), "--host", "127.0.0.1",
                                     "--port", str(port), "--strictPort"], cwd=frozen / "desktop",
                                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                wait_for(base + PAGE, vite)
                receipt.update(run_browser(base, frozen))
                if source_binding() != binding:
                    raise RuntimeError("candidate tree changed while checks ran")
            finally:
                vite.terminate()
                vite.wait(timeout=30)
    except Exception as error:
        receipt["error"] = "; ".join(filter(None, [receipt.get("error"), str(error)]))
    print(json.dumps(receipt, indent=2))
    return 1 if receipt["error"] else 0


if __name__ == "__main__":
    sys.exit(main())
