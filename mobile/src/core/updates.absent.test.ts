import { expect, vi, test } from "vitest";

const native: Record<string, unknown> = {};
let checks = 0;
let installs = 0;
let preferencesWritten = 0;
vi.doMock("expo-modules-core", () => ({ requireNativeModule: () => native }));
vi.doMock("react-native", () => ({ AppState: { currentState: "active", addEventListener: () => { throw new Error("Must not register without native support"); } }, Alert: { alert: () => {} } }));
vi.doMock("./prefs", () => ({ loadPrefs: () => ({}), savePrefs: () => { preferencesWritten++; } }));
const { androidUpdates } = await import("../../modules/beans-core");
const updates = await import("./updates");

test("an absent or incomplete native verifier never checks, offers or installs", async () => {
  const functions = {
    updateCapability: () => "supported",
    updateResult: () => null,
    checkUpdate: async () => { checks++; return null; },
    installUpdate: async () => { installs++; },
    cancelUpdate: () => {},
    addListener: () => ({ remove: () => {} }),
  };
  expect(androidUpdates.capability()).toBe("native_verifier_unavailable");
  for (const missing of Object.keys(functions)) {
    Object.assign(native, functions);
    delete native[missing];
    expect(androidUpdates.capability()).toBe("native_verifier_unavailable");
    updates.initializeUpdates("android", "ai.amoena.beans")();
    await updates.checkUpdates();
    await updates.installUpdate("unverified", true);
    expect(updates.useUpdates.getState().status).toBe("blocked");
  }
  expect(checks).toBe(0);
  expect(installs).toBe(0);
  expect(preferencesWritten).toBe(0);
  Object.assign(native, functions);
  expect(androidUpdates.capability()).toBe("supported");
  native.updateCapability = () => { throw new Error("native source lookup unavailable"); };
  expect(androidUpdates.capability()).toBe("native_verifier_unavailable");
});
