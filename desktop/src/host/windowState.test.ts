import { expect, test } from "vitest";
import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import { fileURLToPath, pathToFileURL } from "node:url";

// A fresh process exercises the native MyGo branch without leaking its import-time
// platform selection into the browser-host store/visibility tests in the full suite.
const here = fileURLToPath(new URL(".", import.meta.url));

interface FixtureOutcome {
  code: number | null;
  signal: NodeJS.Signals | null;
  stdout: string;
  stderr: string;
}

/** The tsx loader, resolved from the installed package and passed as an absolute file URL: a
 * bare `tsx` would depend on the child's cwd and on whatever is on PATH. */
let loader: string | undefined;
function tsxLoader(): string {
  if (!loader) {
    let resolved: string;
    try {
      resolved = createRequire(import.meta.url).resolve("tsx");
    } catch (error) {
      throw new Error(`tsx is not installed for the desktop tests: ${error instanceof Error ? error.message : String(error)}`);
    }
    loader = pathToFileURL(resolved).href;
  }
  return loader;
}

/** Runs the fixture in its own Node process, drains both pipes, and reports how it ended. The
 * child is the one this test owns: on a timeout it is terminated and reaped before failing. */
async function runFixture(script: string, env: NodeJS.ProcessEnv, timeoutMs = 30_000): Promise<FixtureOutcome> {
  const child = spawn(process.execPath, ["--import", tsxLoader(), "--input-type=module", "--eval", script], {
    cwd: here,
    env,
    shell: false,
    stdio: ["ignore", "pipe", "pipe"],
  });
  const stdout: string[] = [];
  const stderr: string[] = [];
  const out = child.stdout;
  const err = child.stderr;
  if (!out || !err) throw new Error("the fixture process has no output pipes");
  out.setEncoding("utf8");
  err.setEncoding("utf8");
  out.on("data", (chunk: string) => { stdout.push(chunk); });
  err.on("data", (chunk: string) => { stderr.push(chunk); });
  let timedOut = false;
  let escalation: NodeJS.Timeout | undefined;
  const deadline = setTimeout(() => {
    timedOut = true;
    child.kill("SIGTERM");
    escalation = setTimeout(() => child.kill("SIGKILL"), 2_000);
  }, timeoutMs);
  let outcome: FixtureOutcome;
  try {
    const { code, signal } = await new Promise<{ code: number | null; signal: NodeJS.Signals | null }>((resolve, reject) => {
      child.once("error", reject);
      child.once("close", (closeCode, closeSignal) => resolve({ code: closeCode, signal: closeSignal }));
    });
    outcome = { code, signal, stdout: stdout.join(""), stderr: stderr.join("") };
  } catch (error) {
    throw new Error(`the fixture process could not run: ${error instanceof Error ? error.message : String(error)}`, { cause: error });
  } finally {
    clearTimeout(deadline);
    if (escalation) clearTimeout(escalation);
  }
  if (timedOut) throw new Error(`the fixture process timed out after ${timeoutMs}ms: ${JSON.stringify(outcome)}`);
  return outcome;
}

test("a disposed native snapshot cannot hide a remounted avatar visibility registry", async () => {
  const script = `
    import assert from "node:assert/strict";
    import { pathToFileURL } from "node:url";
    const base = pathToFileURL(process.cwd() + "/");
    const pending = [], nativeListeners = new Set(), intersections = [];
    globalThis.location = { search: "" };
    const media = Object.assign(new EventTarget(), { matches: false });
    globalThis.window = Object.assign(new EventTarget(), { matchMedia: () => media });
    globalThis.document = Object.assign(new EventTarget(), { visibilityState: "visible" });
    globalThis.IntersectionObserver = class {
      constructor(callback) { intersections.push(callback); }
      observe() {} unobserve() {} disconnect() {}
    };
    globalThis.mygo = {
      call(method) {
        assert.equal(method, "Host.WindowState");
        const { promise, resolve } = Promise.withResolvers();
        pending.push(resolve);
        return promise;
      },
      on(name, listener) {
        nativeListeners.add(listener);
        return () => nativeListeners.delete(listener);
      },
    };
    // Static import cannot select the native branch before the runtime is installed.
    const { watchAvatarVisibility } = await import(new URL("../ui/avatarVisibility.ts", base).href);
    const shown = { focused: true, visible: true, minimized: false, fullScreen: false };
    const hidden = { ...shown, focused: false, visible: false, minimized: true };
    const oldElement = {}, freshElement = {}, fresh = [];
    const stopOld = watchAvatarVisibility(oldElement, () => {});
    intersections[0]([{ target: oldElement, isIntersecting: true, intersectionRatio: 1 }]);
    stopOld();
    const stopFresh = watchAvatarVisibility(freshElement, visible => fresh.push(visible));
    intersections[1]([{ target: freshElement, isIntersecting: true, intersectionRatio: 1 }]);
    pending[1](shown);
    await Promise.resolve(); await Promise.resolve();
    assert.equal(fresh.at(-1), true, "the remounted registry receives its live initial snapshot");
    pending[0](hidden);
    await Promise.resolve(); await Promise.resolve();
    assert.equal(fresh.at(-1), true, "a disposed snapshot must not overwrite the new registry");
    for (const listener of nativeListeners) listener(hidden);
    assert.equal(fresh.at(-1), false, "live native visibility still gates the remounted avatar");
    for (const listener of nativeListeners) listener(shown);
    assert.equal(fresh.at(-1), true);
    stopFresh();
    assert.equal(nativeListeners.size, 0, "teardown releases the native subscription");
  `;
  const env: NodeJS.ProcessEnv = { ...process.env };
  // The fixture must not depend on an inherited loader or on Bun on PATH.
  delete env.NODE_OPTIONS;
  const { code, signal, stdout, stderr } = await runFixture(script, env);
  expect(stderr, `the fixture wrote to stderr:\n${stderr}`).toBe("");
  expect(code, `the fixture exited with ${signal ?? "no signal"}:\n${stdout}`).toBe(0);
  expect(signal).toBeNull();
}, 30_000);

test("native snapshot rejections are reported without notifying live or disposed listeners", async () => {
  const script = `
    import assert from "node:assert/strict";
    import { pathToFileURL } from "node:url";
    const base = pathToFileURL(process.cwd() + "/");
    const pending = [], nativeListeners = new Set(), unhandled = [], reported = [];
    globalThis.location = { search: "" };
    globalThis.mygo = {
      call(method) {
        assert.equal(method, "Host.WindowState");
        const deferred = Promise.withResolvers();
        pending.push(deferred);
        return deferred.promise;
      },
      on(name, listener) {
        nativeListeners.add(listener);
        return () => nativeListeners.delete(listener);
      },
    };
    process.on("unhandledRejection", error => unhandled.push(error));
    console.error = (...args) => reported.push(args);
    const { watchWindowState } = await import(new URL("./index.ts", base).href);
    let oldCalls = 0, liveCalls = 0;
    const stopOld = watchWindowState(() => oldCalls++);
    stopOld();
    const stopLive = watchWindowState(() => liveCalls++);
    const disposedError = new Error("disposed snapshot send failed");
    const liveError = new Error("live snapshot send failed");
    pending[0].reject(disposedError);
    pending[1].reject(liveError);
    // Yield a turn for unhandledRejection delivery, without a wall-clock delay.
    await new Promise(resolve => setImmediate(resolve));
    assert.deepEqual(unhandled, [], "snapshot failure must not create an unhandled rejection");
    assert.equal(oldCalls, 0, "rejection must not notify a disposed listener");
    assert.equal(liveCalls, 0, "rejection must not invent a window state");
    assert.equal(reported.length, 2, "both live and disposed failures remain diagnosable");
    for (const failure of [disposedError, liveError]) {
      assert.ok(reported.some(args =>
        typeof args[0] === "string" && /window.?state/i.test(args[0]) && args.includes(failure)
      ), "diagnostics identify the native window-state operation and original failure");
    }
    for (const listener of nativeListeners) listener({
      focused: true, visible: true, minimized: false, fullScreen: false,
    });
    assert.equal(liveCalls, 1, "live events still work after snapshot failure");
    assert.equal(oldCalls, 0);
    stopLive();
    assert.equal(nativeListeners.size, 0);
  `;
  const env: NodeJS.ProcessEnv = { ...process.env };
  // The fixture must not depend on an inherited loader or on Bun on PATH.
  delete env.NODE_OPTIONS;
  const { code, signal, stdout, stderr } = await runFixture(script, env);
  expect(stderr, `the fixture wrote to stderr:\n${stderr}`).toBe("");
  expect(code, `the fixture exited with ${signal ?? "no signal"}:\n${stdout}`).toBe(0);
  expect(signal).toBeNull();
}, 30_000);
