import { beforeEach, expect, mock, test } from "bun:test";

// The Android app's check for a newer release, against a GitHub API answered here: tags, drafts
// and prerelease releases, the APK and its digest, and the daily check's alert. Also the progress
// helpers and install-failure alert from the update dialog.

let prefs: Record<string, unknown> = {};
const alerts: { title: string; message?: string; buttons: { text: string; onPress?: () => void }[] }[] = [];
const installs: unknown[][] = [];
let installReject: Error | null = null;

mock.module("react-native", () => ({
  Platform: { OS: "android" },
  AppState: { addEventListener: () => {} },
  Alert: {
    alert: (title: string, message?: string, buttons?: (typeof alerts)[number]["buttons"]) => {
      alerts.push({ title, message, buttons: buttons ?? [] });
    },
  },
}));
mock.module("expo-application", () => ({ applicationId: "ai.amoena.beans", nativeApplicationVersion: "1.2.0" }));
mock.module("./prefs", () => ({
  loadPrefs: () => prefs,
  savePrefs: (next: Record<string, unknown>) => {
    prefs = next;
  },
}));
mock.module("../../modules/beans-core", () => ({
  installUpdate: async (...args: unknown[]) => {
    if (installReject) throw installReject;
    installs.push(args);
  },
  onInstallStatus: () => () => {},
}));

/// What GitHub answers, by path under the repository.
let routes: Record<string, unknown> = {};
const githubFetch = (async (input: string | URL) => {
  const path = String(input).replace("https://api.github.com/repos/bloodf/beans", "");
  const body = routes[path];
  if (body === undefined) return new Response("{}", { status: 404 });
  return Response.json(body);
}) as typeof fetch;
globalThis.fetch = githubFetch;

const updates = await import("./updates");

function release(version: string, extra: Record<string, unknown> = {}) {
  return {
    draft: false,
    prerelease: false,
    body: `Notes for ${version}`,
    assets: [
      {
        name: `Beans-${version}.apk`,
        browser_download_url: `https://example.test/${version}.apk`,
        size: 42,
        digest: `sha256:${"ab".repeat(32)}`,
      },
    ],
    ...extra,
  };
}

function tags(...versions: string[]) {
  return versions.map((v) => ({ ref: `refs/tags/beans-v${v}` }));
}

beforeEach(() => {
  prefs = {};
  alerts.length = 0;
  installs.length = 0;
  installReject = null;
  routes = {};
  globalThis.fetch = githubFetch;
  updates.useUpdates.setState({ phase: "idle", latest: undefined, error: undefined, checked: false, progress: 0 });
});

test("versions compare by number", () => {
  expect(updates.isNewer("1.10.0", "1.9.9")).toBe(true);
  expect(updates.isNewer("2.0.0", "1.99.99")).toBe(true);
  expect(updates.isNewer("1.2.0", "1.2.0")).toBe(false);
  expect(updates.isNewer("1.1.9", "1.2.0")).toBe(false);
});

test("updates stay off without both native install functions", () => {
  const present = {
    installUpdate: async () => {},
    onInstallStatus: () => () => {},
  };
  expect(updates.hasNativeUpdater({})).toBe(false);
  expect(updates.hasNativeUpdater({ installUpdate: present.installUpdate })).toBe(false);
  expect(updates.hasNativeUpdater({ onInstallStatus: present.onInstallStatus })).toBe(false);
  expect(updates.hasNativeUpdater(present)).toBe(true);
  expect(updates.computeUpdatesSupported("android", "ai.amoena.beans", {})).toBe(false);
  expect(updates.computeUpdatesSupported("android", "ai.amoena.beans.dev", present)).toBe(false);
  expect(updates.computeUpdatesSupported("ios", "ai.amoena.beans", present)).toBe(false);
  expect(updates.computeUpdatesSupported("android", "ai.amoena.beans", present)).toBe(true);
  expect(updates.updatesSupported).toBe(true);
});

test("download progress is a fraction, zero when the total is unknown", () => {
  expect(updates.downloadProgress(0, 100)).toBe(0);
  expect(updates.downloadProgress(25, 100)).toBe(0.25);
  expect(updates.downloadProgress(50, 0)).toBe(0);
  expect(updates.downloadProgress(10, -1)).toBe(0);
});

test("the progress dialog is active only while downloading or installing", () => {
  expect(updates.isUpdateProgressActive("idle")).toBe(false);
  expect(updates.isUpdateProgressActive("checking")).toBe(false);
  expect(updates.isUpdateProgressActive("downloading")).toBe(true);
  expect(updates.isUpdateProgressActive("installing")).toBe(true);
});

test("the newest published release newer than the build, past drafts and prereleases", async () => {
  routes = {
    "/git/matching-refs/tags/beans-v": tags("1.1.0", "1.2.0", "1.3.0", "1.4.0", "1.5.0", "1.10.0"),
    // 1.10.0 is still a draft, which answers 404 to an unauthenticated read.
    "/releases/tags/beans-v1.5.0": release("1.5.0", { prerelease: true }),
    "/releases/tags/beans-v1.4.0": release("1.4.0", { assets: [] }),
    "/releases/tags/beans-v1.3.0": release("1.3.0"),
    "/releases/tags/beans-v1.1.0": release("1.1.0"),
  };
  expect(await updates.newestRelease()).toEqual({
    version: "1.3.0",
    notes: "Notes for 1.3.0",
    url: "https://example.test/1.3.0.apk",
    sha256: "ab".repeat(32),
    size: 42,
  });
});

test("nothing newer than the build is up to date", async () => {
  routes = { "/git/matching-refs/tags/beans-v": tags("1.1.0", "1.2.0"), "/releases/tags/beans-v1.2.0": release("1.2.0") };
  expect(await updates.newestRelease()).toBeNull();
  await updates.checkForUpdates();
  expect(updates.useUpdates.getState()).toMatchObject({ phase: "idle", latest: undefined, checked: true });
});

test("an APK without a digest is not offered", async () => {
  routes = {
    "/git/matching-refs/tags/beans-v": tags("1.3.0"),
    "/releases/tags/beans-v1.3.0": release("1.3.0", {
      assets: [{ name: "Beans-1.3.0.apk", browser_download_url: "x", size: 1, digest: null }],
    }),
  };
  expect(await updates.newestRelease()).toBeNull();
});

test("the daily check offers a release once, and Skip This Version keeps it from asking again", async () => {
  routes = { "/git/matching-refs/tags/beans-v": tags("1.3.0"), "/releases/tags/beans-v1.3.0": release("1.3.0") };
  await updates.checkForUpdates(true);
  expect(updates.useUpdates.getState().latest?.version).toBe("1.3.0");
  expect(prefs.update_checked_at).toBeNumber();
  expect(alerts.map((a) => a.title)).toEqual(["Beans 1.3.0 is available"]);
  expect(alerts[0]!.buttons.map((b) => b.text)).toEqual(["Skip This Version", "Remind Me Later", "Install Update"]);

  alerts[0]!.buttons[0]!.onPress!();
  expect(prefs.update_skipped).toBe("1.3.0");
  await updates.checkForUpdates(true);
  expect(alerts).toHaveLength(1);
  // A check of the user's own still finds it, for Settings to install.
  await updates.checkForUpdates();
  expect(updates.useUpdates.getState().latest?.version).toBe("1.3.0");
});

test("Install Update hands the APK, its digest, and its size to the native module", async () => {
  routes = { "/git/matching-refs/tags/beans-v": tags("1.3.0"), "/releases/tags/beans-v1.3.0": release("1.3.0") };
  await updates.checkForUpdates(true);
  alerts[0]!.buttons[2]!.onPress!();
  await Promise.resolve();
  expect(installs).toEqual([["https://example.test/1.3.0.apk", "ab".repeat(32), 42]]);
  expect(updates.useUpdates.getState().phase).toBe("downloading");
});

test("a failed install returns to idle and alerts with the reason", async () => {
  installReject = new Error("digest mismatch");
  await updates.installUpdate({
    version: "1.3.0",
    notes: "",
    url: "https://example.test/1.3.0.apk",
    sha256: "ab".repeat(32),
    size: 42,
  });
  expect(updates.useUpdates.getState()).toMatchObject({ phase: "idle", error: "digest mismatch" });
  expect(alerts.map((a) => a.title)).toEqual(["Couldn’t install the update"]);
  expect(alerts[0]!.message).toBe("digest mismatch");
});

test("a failed check keeps its reason for Settings", async () => {
  globalThis.fetch = (async () => new Response("{}", { status: 403 })) as unknown as typeof fetch;
  await updates.checkForUpdates();
  expect(updates.useUpdates.getState()).toMatchObject({ phase: "idle", error: "GitHub answered 403", checked: true });
});
