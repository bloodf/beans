import { describe, expect, test, spyOn } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { readVersion } from "./app.ts";
import { downloadArtifact, runEasRelease, stageEasBuild, validateEasBuild } from "./release-eas.ts";
import { androidNdk, hostLibrary } from "./release-build-inputs.ts";

const revision = "a".repeat(40);
const project = "11111111-1111-4111-8111-111111111111";
const build = { id: "22222222-2222-4222-8222-222222222222", status: "FINISHED", platform: "ANDROID", appIdentifier: "ai.amoena.beans", distribution: "STORE", buildProfile: "github", gitCommitHash: revision, appVersion: "1.0.11", appBuildVersion: "42", app: { id: project }, artifacts: { buildUrl: "https://artifacts.eascdn.net/build.apk" } };
test("downloads and stages exact finished APK/AAB/store IPA with hashes, never URLs", async () => {
  const directory = await mkdtemp(join(tmpdir(), "beans-eas-test-"));
  const previous = process.env.BEANS_EXPO_PROJECT_ID;
  process.env.BEANS_EXPO_PROJECT_ID = project;
  const fetcher = spyOn(globalThis, "fetch").mockImplementation(async () => new Response(new Uint8Array([0x50, 0x4b, 3, 4, 1])) as any);
  try {
    for (const [profile, platform, suffix] of [["github", "ANDROID", ".apk"], ["production", "ANDROID", ".aab"], ["testflight", "IOS", "-store.ipa"]] as const) {
      await stageEasBuild({ ...build, appVersion: readVersion(), platform, buildProfile: profile }, profile, revision, directory);
      const proof = await Bun.file(join(directory, `eas-${profile}.json`)).json();
      expect(proof.artifact).toBe(`Beans-${readVersion()}${suffix}`);
      expect(proof.sha256).toHaveLength(64);
      expect(proof.revision).toBe(revision);
      expect(JSON.stringify(proof)).not.toContain("https");
    }
    const oldToken = process.env.EXPO_TOKEN;
    const oldOwner = process.env.BEANS_EXPO_OWNER;
    process.env.EXPO_TOKEN = "test-token";
    process.env.BEANS_EXPO_OWNER = "test-owner";
    try {
      const git = (args: string[]) => ({ exitCode: 0, stdout: Buffer.from(args.includes("rev-parse") ? revision : "") }) as any;
      await runEasRelease(["build", "github", revision, join(directory, "fresh")], async (args) => { expect(args).toContain("--wait"); return [{ ...build, appVersion: readVersion() }]; }, git);
      await runEasRelease(["build", "github", revision, join(directory, "retry"), build.id], async (args) => { expect(args[0]).toBe("build:view"); return { ...build, appVersion: readVersion() }; }, git);
      await expect(runEasRelease(["build", "github", revision, directory], async () => [], git)).rejects.toThrow("exactly one");
      await expect(runEasRelease(["build", "github", revision, directory], async () => [], () => ({ exitCode: 0, stdout: Buffer.from("dirty") }) as any)).rejects.toThrow("clean");
      await expect(runEasRelease(["bad"], async () => [], git)).rejects.toThrow("usage");
      delete process.env.EXPO_TOKEN;
      await expect(runEasRelease(["build", "github", revision, directory], async () => [], git)).rejects.toThrow("required");
    } finally {
      if (oldToken === undefined) delete process.env.EXPO_TOKEN; else process.env.EXPO_TOKEN = oldToken;
      if (oldOwner === undefined) delete process.env.BEANS_EXPO_OWNER; else process.env.BEANS_EXPO_OWNER = oldOwner;
    }
    await expect(stageEasBuild({ ...build, status: "ERRORED" }, "github", revision, directory)).rejects.toThrow();
    const redirects = ["https://api.expo.dev/builds/artifact", "https://wf-artifacts.eascdn.net/build.ipa"];
    fetcher.mockImplementation(async () => redirects.length
      ? new Response(null, { status: 307, headers: { location: redirects.shift()! } }) as any
      : new Response(new Uint8Array([0x50, 0x4b, 3, 4, 1])) as any);
    expect((await downloadArtifact("https://expo.dev/artifacts/build.ipa")).length).toBe(5);
    fetcher.mockImplementation(async () => new Response(null, { status: 307, headers: { location: "https://evil.eascdn.net/build.ipa" } }) as any);
    await expect(downloadArtifact(build.artifacts.buildUrl)).rejects.toThrow("trusted HTTPS");
    fetcher.mockImplementation(async () => new Response("denied", { status: 403 }) as any);
    await expect(downloadArtifact(build.artifacts.buildUrl)).rejects.toThrow("403");
    fetcher.mockImplementation(async () => new Response(null, { status: 302, headers: { location: "http://localhost/secret" } }) as any);
    await expect(downloadArtifact(build.artifacts.buildUrl)).rejects.toThrow("HTTPS");
    fetcher.mockImplementation(async () => new Response(null, { status: 302, headers: { location: build.artifacts.buildUrl } }) as any);
    await expect(downloadArtifact(build.artifacts.buildUrl)).rejects.toThrow("redirect");
    fetcher.mockImplementation(async () => new Response("not a zip") as any);
    await expect(downloadArtifact(build.artifacts.buildUrl)).rejects.toThrow("ZIP");
    await expect(downloadArtifact("http://localhost/evil")).rejects.toThrow("HTTPS");
  } finally {
    fetcher.mockRestore();
    if (previous === undefined) delete process.env.BEANS_EXPO_PROJECT_ID; else process.env.BEANS_EXPO_PROJECT_ID = previous;
    await rm(directory, { recursive: true, force: true });
  }
});
describe("native build inputs", () => {
  test("host suffix", () => { expect(hostLibrary("linux")).toBe("liblorca_mobile.so"); expect(hostLibrary("darwin")).toBe("liblorca_mobile.dylib"); expect(() => hostLibrary("win32")).toThrow(); });
  test("explicit NDK inputs", () => {
    expect(androidNdk({ ANDROID_NDK_ROOT: "/ndk" }, "linux", () => true)).toBe("/ndk");
    expect(androidNdk({ ANDROID_SDK_ROOT: "/sdk", BEANS_ANDROID_NDK_VERSION: "27.1.12297006" }, "linux", () => true)).toBe("/sdk/ndk/27.1.12297006");
    expect(() => androidNdk({}, "linux", () => true)).toThrow();
    expect(() => androidNdk({ ANDROID_NDK_HOME: "/missing" }, "linux", () => false)).toThrow();
    expect(() => androidNdk({ BEANS_ANDROID_NDK_VERSION: "../../evil" }, "linux", () => true)).toThrow();
  });
});
describe("EAS provenance", () => {
  test("AAB, store IPA and retry numbers", () => {
    for (const [profile, platform] of [["production", "ANDROID"], ["testflight", "IOS"]] as const) {
      for (const number of ["43", "44"]) expect(validateEasBuild({ ...build, platform, buildProfile: profile, appBuildVersion: number }, profile, revision, "1.0.11", project).appBuildVersion).toBe(number);
    }
  });
  test("accepts only finished exact-source build", () => expect(validateEasBuild(build, "github", revision, "1.0.11", project)).toEqual(build));
  for (const invalid of [{ status: "IN_PROGRESS" }, { gitCommitHash: "b".repeat(40) }, { app: { id: "other" } }, { appVersion: "1.0.12" }, { platform: "IOS" }, { buildProfile: "production" }, { appBuildVersion: "0" }, { artifacts: { buildUrl: "http://artifacts.eascdn.net/build.apk" } }, { artifacts: { buildUrl: "https://localhost/build.apk" } }]) {
    test(`rejects ${JSON.stringify(invalid)}`, () => expect(() => validateEasBuild({ ...build, ...invalid }, "github", revision, "1.0.11", project)).toThrow());
  }
});
