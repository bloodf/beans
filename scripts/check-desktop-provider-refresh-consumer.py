#!/usr/bin/env python3
"""Beans desktop mounted custom-provider consumer checks.

Serves desktop/ with its own Vite dev server, opens the mounted consumer check page in a real
Chromium and fails when any check errors. The check itself lives next to the sheet it exercises, so
this script starts no framework, copies no oracle and runs nothing else. It follows
`scripts/check-desktop-skills-consumer.py`.

Requires `bun install` in desktop/, the generated desktop/src/mygo.ts (gitignored; `bun run generate`),
and the playwright Python package with its chromium browser.
"""

import hashlib
import json
import os
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request

DESKTOP = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "desktop")
PAGE = "/src/ui/sheets/customProviderMountedCheck.html"
TIMEOUT_SECONDS = 120

def free_port() -> int:
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]

def wait_for(url: str, process: subprocess.Popen) -> None:
    deadline = time.time() + TIMEOUT_SECONDS
    while time.time() < deadline:
        if process.poll() is not None:
            raise SystemExit(f"vite exited with {process.returncode} before serving {url}")
        try:
            with urllib.request.urlopen(url, timeout=2):
                return
        except (urllib.error.URLError, OSError):
            time.sleep(0.2)
    raise SystemExit(f"vite did not serve {url} within {TIMEOUT_SECONDS}s")

def source_binding() -> dict:
    """The commit and the exact bytes of every module the check exercises, so a receipt
    names what ran rather than what the branch is believed to contain."""
    root = os.path.dirname(DESKTOP)
    modules = [
        "src/ui/sheets/customProviderMountedCheck.ts",
        "src/ui/sheets/customProvider.test.tsx",
        "src/ui/sheets/customProvider.tsx",
        "src/model/store.ts",
        "src/model/models.ts",
    ]
    binding = {"commit": subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=root, capture_output=True, text=True).stdout.strip()}
    for module in modules:
        with open(os.path.join(DESKTOP, module), "rb") as handle:
            binding[module] = hashlib.sha256(handle.read()).hexdigest()
    return binding

def main() -> int:
    generated = os.path.join(DESKTOP, "src", "mygo.ts")
    if not os.path.exists(generated):
        raise SystemExit("desktop/src/mygo.ts is missing; run `bun run generate` in desktop/ first")
    port = free_port()
    vite = subprocess.Popen(
        ["bun", "node_modules/vite/bin/vite.js", "--port", str(port), "--strictPort"],
        cwd=DESKTOP, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        base = f"http://localhost:{port}"
        wait_for(base + PAGE, vite)
        from playwright.sync_api import sync_playwright

        console: list[str] = []
        with sync_playwright() as playwright:
            browser = playwright.chromium.launch()
            page = browser.new_page()
            page.on("console", lambda message: console.append(f"{message.type}: {message.text}"))
            page.goto(base + PAGE, wait_until="load")
            try:
                page.wait_for_function("window.__result !== undefined || window.__error !== undefined", timeout=60000)
            except Exception:
                print(json.dumps({"result": None, "error": "checks did not settle", "console": console}, indent=2))
                return 1
            result = page.evaluate("window.__result")
            error = page.evaluate("window.__error")
            browser.close()
    finally:
        vite.terminate()
        vite.wait(timeout=30)

    # Every check must have reported; an empty page is not a pass.
    missing = [name for name, _ in [("refresh", None)] if not (result or {}).get(name)]
    if missing:
        error = "; ".join(filter(None, [error, f"no result reported for: {', '.join(missing)}"]))
    print(json.dumps({"source": source_binding(), "result": result, "error": error, "console": console}, indent=2))
    return 1 if error else 0

if __name__ == "__main__":
    sys.exit(main())