import { expect, test } from "bun:test";

// Each child gets its own import-time browser host state. WebSocket construction is
// forbidden, so an incompatible setting cannot contact any installed CLI.
test("invalid browser ports stay offline through load, preference edits and reconnect", async () => {
  const script = `
    import assert from "node:assert/strict";
    globalThis.location = { search: process.env.CASE_QUERY };
    Object.defineProperty(globalThis, "navigator", { configurable: true, value: { language: "en", userAgent: "test" } });
    globalThis.window = new EventTarget();
    globalThis.document = { visibilityState: "visible" };
    let writes = 0, sockets = 0;
    globalThis.localStorage = {
      getItem: () => process.env.CASE_SAVED,
      setItem: () => { writes++; },
    };
    globalThis.WebSocket = class {
      constructor() { sockets++; throw new Error("must not dial an incompatible endpoint"); }
    };
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
  for (const [query, saved] of [
    ["?port=4864", '{"cliPort":4875}'],
    ["?port=garbage", '{"cliPort":4875}'],
    ["?port=", '{"cliPort":4875}'],
    ["?port=4864&port=4875", '{"cliPort":4875}'],
    ["?port=4875", '{"cliPort":4864}'],
    ["", '{"cliPort":"4875"}'],
    ["", '{"cliPort":'],
  ]) {
    const child = Bun.spawn([process.execPath, "--eval", script], {
      cwd: import.meta.dir,
      env: { ...process.env, CASE_QUERY: query, CASE_SAVED: saved, HOST_MODULE: `${import.meta.dir}/index.ts` },
      stdout: "pipe", stderr: "pipe",
    });
    const [code, stderr] = await Promise.all([child.exited, new Response(child.stderr).text()]);
    expect(stderr).toBe("");
    expect(code).toBe(0);
  }
});
