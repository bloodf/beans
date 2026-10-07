import { expect, test } from "bun:test";

// A fresh process exercises the native MyGo branch without leaking its import-time
// platform selection into the browser-host store/visibility tests in the full suite.
test("a disposed native snapshot cannot hide a remounted avatar visibility registry", async () => {
  const script = `
    import assert from "node:assert/strict";
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
    const { watchAvatarVisibility } = await import("../ui/avatarVisibility.ts");
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
  const child = Bun.spawn([process.execPath, "--eval", script], { cwd: import.meta.dir, stdout: "pipe", stderr: "pipe" });
  const [code, stderr] = await Promise.all([child.exited, new Response(child.stderr).text()]);
  expect(stderr).toBe("");
  expect(code).toBe(0);
});

test("native snapshot rejections are reported without notifying live or disposed listeners", async () => {
  const script = `
    import assert from "node:assert/strict";
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
    const { watchWindowState } = await import("./index.ts");
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
  const child = Bun.spawn([process.execPath, "--eval", script], { cwd: import.meta.dir, stdout: "pipe", stderr: "pipe" });
  const [code, stderr] = await Promise.all([child.exited, new Response(child.stderr).text()]);
  expect(stderr).toBe("");
  expect(code).toBe(0);
});
