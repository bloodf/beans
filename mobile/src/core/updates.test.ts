import { setTimeout as delay } from "node:timers/promises";
import { afterEach, beforeEach, expect, vi, test } from "vitest";
import type { UpdateProgress } from "../../modules/beans-core";

let prefs: Record<string, unknown> = {};
let capability = "supported";
let nextOffer = { id: "consent-1", version: "1.10.0", notes: "Release notes are not authority" };
let check: () => Promise<typeof nextOffer | null> = async () => nextOffer;
let install: (id: string) => Promise<void> = async () => {};
let checks = 0;
let cancels = 0;
let committed = false;
let installs: string[] = [];
let drafts = false;
let draftChanged = () => {};
let nativeResult: string | null = null;
let progress: (event: UpdateProgress) => void = () => {};
let appChanged: (state: string) => void = () => {};
const appState = { currentState: "active", addEventListener: (_event: string, listener: typeof appChanged) => { appChanged = listener; return { remove: () => {} }; } };
const alerts: { title: string; message: string; buttons: { onPress?: () => void }[] }[] = [];
vi.doMock("react-native", () => ({ AppState: appState, Alert: { alert: (title: string, message: string, buttons = []) => alerts.push({ title, message, buttons }) } }));
vi.doMock("./prefs", () => ({ loadPrefs: () => ({ ...prefs }), savePrefs: (value: Record<string, unknown>) => { prefs = value; } }));
vi.doMock("./updateDrafts", () => ({ hasUpdateDrafts: () => drafts, subscribeComposerDrafts: (listener: () => void) => { draftChanged = listener; return () => { draftChanged = () => {}; }; } }));
vi.doMock("../../modules/beans-core", () => ({ androidUpdates: {
  capability: () => capability,
  check: async () => { checks++; return check(); },
  install: async (id: string) => { installs.push(id); return install(id); },
  cancel: () => { cancels++; return committed ? "committed" : "cancelled"; },
  onProgress: (listener: typeof progress) => { progress = listener; return () => {}; },
  result: () => { const result = nativeResult; nativeResult = null; return result; },
} }));
const updates = await import("./updates");
let dispose = () => {};
const settle = async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); };
const background = () => { appState.currentState = "background"; appChanged("background"); };
const foreground = () => { appState.currentState = "active"; appChanged("active"); };
const event = (phase: UpdateProgress["phase"], downloaded = 25, id = nextOffer.id) => progress({ id, phase, downloaded, total: 100 });
function pending<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
beforeEach(() => {
  prefs = { update_checks: false };
  capability = "supported";
  nextOffer = { id: "consent-1", version: "1.10.0", notes: "Release notes are not authority" };
  check = async () => nextOffer;
  install = async () => {};
  checks = 0; cancels = 0; committed = false; installs = []; drafts = false; nativeResult = null; alerts.length = 0;
  appState.currentState = "active";
  dispose = updates.initializeUpdates("android", "ai.amoena.beans");
});
afterEach(() => dispose());

test("native eligibility excludes iOS, Dev, unknown packages and store distributions", () => {
  expect(updates.updateBlock("ios", "ai.amoena.beans")).toBe("store_managed");
  expect(updates.updateBlock("android", "ai.amoena.beans.dev")).toBe("development");
  expect(updates.updateBlock("android", null)).toBe("store_managed");
  expect(updates.updateBlock("android", "other.package")).toBe("store_managed");
  capability = "store_managed";
  expect(updates.updateBlock("android", "ai.amoena.beans")).toBe("store_managed");
  capability = "native_verifier_unavailable";
  expect(updates.updateBlock("android", "ai.amoena.beans")).toBe("native_verifier_unavailable");
});

test("daily checks offer only a native verified snapshot and never install", async () => {
  expect(checks).toBe(0);
  prefs = {};
  await updates.checkUpdates(false);
  expect(checks).toBe(1);
  expect(installs).toEqual([]);
  expect(updates.useUpdates.getState()).toMatchObject({ status: "offered", offer: nextOffer });
  expect(alerts[0]!.title).toBe("Beans 1.10.0 is available");
  expect(alerts[0]!.message).toContain("Installing replaces this process");
  await updates.checkUpdates(false);
  expect(checks).toBe(1);
  alerts[0]!.buttons[2]!.onPress!();
  await settle();
  expect(installs).toEqual(["consent-1"]);
  expect(updates.useUpdates.getState()).toMatchObject({ status: "confirmation", installingVersion: "1.10.0", offer: null });
});

test("Skip and Later consume only their offer and manual checks override exact skip", async () => {
  prefs = {};
  await updates.checkUpdates(false);
  alerts[0]!.buttons[0]!.onPress!();
  expect(prefs.update_skipped_version).toBe("1.10.0");
  expect(installs).toEqual([]);
  prefs.update_checked_at = 0;
  await updates.checkUpdates(false);
  expect(alerts).toHaveLength(1);
  expect(updates.useUpdates.getState().offer).toBeNull();
  await updates.checkUpdates();
  expect(updates.useUpdates.getState().offer?.id).toBe("consent-1");
  nextOffer = { ...nextOffer, id: "consent-2" };
  await updates.checkUpdates();
  alerts[0]!.buttons[1]!.onPress!();
  alerts[0]!.buttons[2]!.onPress!();
  await settle();
  expect(updates.useUpdates.getState().offer?.id).toBe("consent-2");
  expect(installs).toEqual([]);
});

test("unsaved drafts and unconfirmed saves block replacement without consuming the offer", async () => {
  await updates.checkUpdates();
  drafts = true;
  await updates.installUpdate(nextOffer.id, true);
  drafts = false;
  await updates.installUpdate(nextOffer.id, false);
  expect(installs).toEqual([]);
  expect(updates.useUpdates.getState()).toMatchObject({ status: "error", offer: nextOffer });
  expect(alerts).toHaveLength(2);
});

test("progress is finite, bounded, monotonic and tied to the active consent id", async () => {
  await updates.checkUpdates();
  const pendingInstall = pending<void>();
  install = () => pendingInstall.promise;
  const run = updates.installUpdate(nextOffer.id, true);
  event("downloading", 80, "old-consent");
  expect(updates.useUpdates.getState().progress).toBe(0);
  event("downloading", 25);
  event("downloading", 10);
  event("downloading", NaN);
  expect(updates.useUpdates.getState().progress).toBe(0.25);
  event("downloading", 200);
  expect(updates.useUpdates.getState().progress).toBe(1);
  event("installing");
  event("downloading", 10);
  expect(updates.useUpdates.getState().status).toBe("installing");
  pendingInstall.resolve();
  await run;
  expect(updates.useUpdates.getState().status).toBe("confirmation");
});

test("double install taps do not consume consent twice or reset the active dialog", async () => {
  await updates.checkUpdates();
  const pendingInstall = pending<void>();
  install = () => pendingInstall.promise;
  const run = updates.installUpdate(nextOffer.id, true);
  event("downloading", 40);
  await updates.installUpdate(nextOffer.id, true);
  await updates.checkUpdates();
  expect(installs).toEqual(["consent-1"]);
  expect(checks).toBe(1);
  expect(updates.useUpdates.getState()).toMatchObject({ status: "downloading", progress: 0.4 });
  pendingInstall.resolve(); await run;
});

test("background cancels download and late success or progress cannot resurrect it", async () => {
  await updates.checkUpdates();
  const pendingInstall = pending<void>();
  install = () => pendingInstall.promise;
  const run = updates.installUpdate(nextOffer.id, true);
  background();
  expect(cancels).toBe(1);
  event("downloading", 80);
  event("installing");
  pendingInstall.resolve(); await run;
  expect(updates.useUpdates.getState()).toMatchObject({ status: "idle", progress: 0, installingVersion: null });
  expect(alerts).toHaveLength(0);
});

test("new drafts cancel an uncommitted download and its late error is ignored", async () => {
  await updates.checkUpdates();
  const pendingInstall = pending<void>();
  install = () => pendingInstall.promise;
  const run = updates.installUpdate(nextOffer.id, true);
  drafts = true; draftChanged();
  expect(cancels).toBe(1);
  expect(updates.useUpdates.getState().status).toBe("error");
  pendingInstall.reject(new Error("transport cancelled")); await run;
  event("installing");
  expect(alerts).toHaveLength(1);
  expect(alerts[0]!.message).toBe("Save or send drafts before installing");
});

test("background expires in-flight discovery and suppresses its late failure", async () => {
  const pendingCheck = pending<typeof nextOffer | null>();
  check = () => pendingCheck.promise;
  const run = updates.checkUpdates();
  background();
  pendingCheck.reject(new Error("offline")); await run;
  expect(updates.useUpdates.getState()).toMatchObject({ status: "idle", offer: null, error: null });
  expect(alerts).toHaveLength(0);
});

test("committed installation survives background and system denial clears the dialog", async () => {
  await updates.checkUpdates();
  const pendingInstall = pending<void>();
  install = () => pendingInstall.promise;
  const run = updates.installUpdate(nextOffer.id, true);
  event("installing");
  background();
  expect(cancels).toBe(0);
  pendingInstall.resolve(); await run;
  expect(updates.useUpdates.getState().status).toBe("confirmation");
  nativeResult = "Installation cancelled or failed. Check and consent again.";
  foreground();
  expect(updates.useUpdates.getState().status).toBe("error");
  event("installing");
  expect(updates.useUpdates.getState().status).toBe("error");
  expect(alerts).toHaveLength(1);
});

test("native failure closes progress and cannot be reopened by late events", async () => {
  await updates.checkUpdates();
  install = async () => { throw new Error("APK signer incompatible with installed Beans"); };
  await updates.installUpdate(nextOffer.id, true);
  event("installing");
  expect(updates.useUpdates.getState()).toMatchObject({ status: "error", offer: null, installingVersion: null });
  expect(alerts[0]!.message).toBe("APK signer incompatible with installed Beans");
  await updates.installUpdate(nextOffer.id, true);
  expect(installs).toHaveLength(1);
});

test("system completion while foreground is observed without a focus transition", async () => {
  await updates.checkUpdates();
  await updates.installUpdate(nextOffer.id, true);
  nativeResult = "Immediate package installer failure";
  await delay(1100);
  expect(updates.useUpdates.getState().status).toBe("error");
  expect(alerts[0]!.message).toBe("Immediate package installer failure");
});

test("installed result resets progress and opt-out preserves unrelated preferences", async () => {
  prefs.app_lang = "zh";
  updates.setUpdateChecks(true);
  expect(prefs.app_lang).toBe("zh");
  expect(prefs.update_checks).toBe(true);
  await updates.checkUpdates();
  nativeResult = "installed";
  await updates.installUpdate(nextOffer.id, true);
  expect(updates.useUpdates.getState()).toMatchObject({ status: "idle", progress: 0, installingVersion: null });
});

test("progress helpers reject invalid values and preserve installer confirmation", () => {
  expect(updates.downloadProgress(25, 100)).toBe(0.25);
  for (const [downloaded, total] of [[NaN, 100], [20, Infinity], [10, 0], [-1, 100]]) expect(updates.downloadProgress(downloaded!, total!)).toBe(0);
  expect(updates.downloadProgress(200, 100)).toBe(1);
  for (const phase of ["blocked", "idle", "checking", "offered", "error"] as const) expect(updates.isUpdateProgressActive(phase)).toBe(false);
  for (const phase of ["downloading", "installing", "confirmation"] as const) expect(updates.isUpdateProgressActive(phase)).toBe(true);
});


test("commit wins cancellation even when its progress event arrives after background", async () => {
  await updates.checkUpdates();
  const pendingInstall = pending<void>();
  install = () => pendingInstall.promise;
  const run = updates.installUpdate(nextOffer.id, true);
  committed = true;
  background();
  expect(updates.useUpdates.getState()).toMatchObject({ status: "confirmation", installingId: nextOffer.id });
  event("installing");
  pendingInstall.resolve(); await run;
  expect(updates.useUpdates.getState().status).toBe("confirmation");
  foreground();
  nativeResult = "Delayed installer rejection";
  await delay(1100);
  expect(updates.useUpdates.getState().status).toBe("error");
  expect(alerts[0]!.message).toBe("Delayed installer rejection");
});

test("drafts created after native commit do not claim a successful cancellation", async () => {
  await updates.checkUpdates();
  const pendingInstall = pending<void>();
  install = () => pendingInstall.promise;
  const run = updates.installUpdate(nextOffer.id, true);
  committed = true;
  drafts = true; draftChanged();
  expect(updates.useUpdates.getState().status).toBe("confirmation");
  expect(alerts[0]!.message).toContain("Cancel it to keep new drafts");
  pendingInstall.resolve(); await run;
  expect(updates.useUpdates.getState().status).toBe("confirmation");
});
