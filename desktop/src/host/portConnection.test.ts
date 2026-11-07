import { expect, test } from "vitest";
import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import { join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

// Each child gets its own import-time browser host state. WebSocket construction is
// forbidden, so an incompatible setting cannot contact any installed CLI.
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

test("invalid browser ports stay offline through load, preference edits and reconnect", async () => {
  const script = `
    import assert from "node:assert/strict";
    globalThis.location = { search: process.env.CASE_QUERY };
    Object.defineProperty(globalThis, "navigator", { configurable: true, value: { language: "en", userAgent: "test" } });
    globalThis.window = new EventTarget();
    globalThis.document = { visibilityState: "visible" };
    let writes = 0, sockets = 0;
    Object.defineProperty(globalThis, "localStorage", { configurable: true, writable: true, value: {
      getItem: () => process.env.CASE_SAVED,
      setItem: () => { writes++; },
    } });
    Object.defineProperty(globalThis, "WebSocket", { configurable: true, writable: true, value: class {
      constructor() { sockets++; throw new Error("must not dial an incompatible endpoint"); }
    } });
    const hostModule = process.env.HOST_MODULE;
    // Static import would run before the browser globals that select this host branch.
    const { loadHost, cliTransport, setPreferences, preferences } = await import(hostModule);
    const connection = cliTransport();
    assert.equal(sockets, 0, "the host must load saved configuration before any dial");
    await loadHost();
    const state = await connection.state();
    assert.equal(state.connection, "disconnected");
    assert.equal(state.launcher.kind, "failed");
    assert.equal(state.launcher.failure.kind, "configuration");
    connection.reconnect();
    const before = preferences();
    await assert.rejects(setPreferences({ cliPort: 4864 }));
    assert.deepEqual(preferences(), before);
    assert.equal(writes, 0, "a rejected port must not overwrite saved preferences");
    assert.equal(sockets, 0, "rejection must happen before WebSocket construction");
  `;
  const hostModule = pathToFileURL(join(here, "index.ts")).href;
  for (const [query, saved] of [
    ["?port=4864", '{"cliPort":4875}'],
    ["?port=garbage", '{"cliPort":4875}'],
    ["?port=", '{"cliPort":4875}'],
    ["?port=4864&port=4875", '{"cliPort":4875}'],
    ["?port=4875", '{"cliPort":4864}'],
    ["", '{"cliPort":"4875"}'],
    ["", '{"cliPort":'],
  ]) {
    const env: NodeJS.ProcessEnv = { ...process.env, CASE_QUERY: query, CASE_SAVED: saved, HOST_MODULE: hostModule };
    // The fixture must not depend on an inherited loader or on Bun on PATH.
    delete env.NODE_OPTIONS;
    const { code, signal, stdout, stderr } = await runFixture(script, env);
    expect(stderr, `the fixture wrote to stderr:\n${stderr}`).toBe("");
    expect(code, `the fixture exited with ${signal ?? "no signal"}:\n${stdout}`).toBe(0);
    expect(signal).toBeNull();
  }
}, 60_000);
