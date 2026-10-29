import { expect, mock, test } from "bun:test";

// When beans-core has no installUpdate/onInstallStatus, the Android production build must not
// check GitHub or offer an update.

let prefs: Record<string, unknown> = {};
const alerts: unknown[] = [];
let fetches = 0;

mock.module("react-native", () => ({
  Platform: { OS: "android" },
  AppState: { addEventListener: () => {} },
  Alert: { alert: (...args: unknown[]) => alerts.push(args) },
}));
mock.module("expo-application", () => ({ applicationId: "ai.amoena.beans", nativeApplicationVersion: "1.2.0" }));
mock.module("./prefs", () => ({
  loadPrefs: () => prefs,
  savePrefs: (next: Record<string, unknown>) => {
    prefs = next;
  },
}));
// Real beans-core at this slice: no Android installer exports.
mock.module("../../modules/beans-core", () => ({
  start: () => {},
  request: async () => ({}),
  onEvent: () => () => {},
  wake: () => {},
  setOpenChat: () => {},
}));

globalThis.fetch = (async () => {
  fetches += 1;
  return Response.json([]);
}) as typeof fetch;

const updates = await import("./updates");

test("a build without the native updater never offers or checks for updates", async () => {
  expect(updates.hasNativeUpdater()).toBe(false);
  expect(updates.updatesSupported).toBe(false);
  expect(updates.computeUpdatesSupported("android", "ai.amoena.beans")).toBe(false);

  updates.startUpdateChecks();
  await updates.checkForUpdates(true);

  expect(fetches).toBe(0);
  expect(prefs.update_checked_at).toBeUndefined();
  expect(alerts).toHaveLength(0);
  const idle = updates.useUpdates.getState();
  expect(idle.phase).toBe("idle");
  expect(idle.checked).toBe(false);
  expect(idle.latest).toBeUndefined();
  expect(idle.error).toBeUndefined();

  // A direct install still fails closed with an alert rather than calling a missing native method.
  await updates.installUpdate({
    version: "1.3.0",
    notes: "",
    url: "https://example.test/1.3.0.apk",
    sha256: "ab".repeat(32),
    size: 42,
  });
  expect(alerts).toHaveLength(1);
  expect(updates.useUpdates.getState()).toMatchObject({ phase: "idle", error: "This build cannot install updates" });
});
