import * as trust from "./release-build-inputs.ts";
import { readFileSync } from "node:fs";
import { existsSync } from "node:fs";
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
import { describe, expect, test, vi } from "vitest";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { readVersion } from "./app.ts";
import { downloadArtifact, runEasRelease, runLocalEasRelease, stageEasBuild, stageLocalBuild, stagePair, validateEasBuild } from "./release-eas.ts";
import { androidNdk, hostLibrary } from "./release-build-inputs.ts";
import { archiveFixture } from "./release-archive-fixture.ts";

const revision = "a".repeat(40);
const project = "11111111-1111-4111-8111-111111111111";
const build = { id: "22222222-2222-4222-8222-222222222222", status: "FINISHED", platform: "ANDROID", appIdentifier: "ai.amoena.beans", distribution: "STORE", buildProfile: "github", gitCommitHash: revision, appVersion: "1.0.11", appBuildVersion: "42", app: { id: project }, artifacts: { buildUrl: "https://artifacts.eascdn.net/build.apk" } };
test("missing public policy rejects staging and cloud calls before transport or credentials", async () => {
  const directory = await mkdtemp(join(tmpdir(), "beans-eas-test-"));
  const previous = process.env.BEANS_EXPO_PROJECT_ID;
  process.env.BEANS_EXPO_PROJECT_ID = project;
  const payload = archiveFixture();
  const fetcher = vi.spyOn(globalThis, "fetch").mockImplementation(async () => new Response(payload) as any);
  try {
    for (const [profile, platform, suffix] of [["github", "ANDROID", ".apk"], ["production", "ANDROID", ".aab"], ["testflight", "IOS", "-store.ipa"]] as const) {
      await expect(stageEasBuild({ ...build, appVersion: readVersion(), platform, buildProfile: profile }, profile, revision, directory)).rejects.toThrow("policy unavailable");
      expect(existsSync(join(directory, `eas-${profile}.json`))).toBe(false);
      expect(existsSync(join(directory, `Beans-${readVersion()}${suffix}`))).toBe(false);
    }
    const oldToken = process.env.EXPO_TOKEN;
    const oldOwner = process.env.BEANS_EXPO_OWNER;
    process.env.EXPO_TOKEN = "test-token";
    process.env.BEANS_EXPO_OWNER = "test-owner";
    try {
      const git = (args: string[]) => ({ exitCode: 0, stdout: Buffer.from(args.includes("rev-parse") ? revision : "") }) as any;
      let queried = false;
      const query = async () => { queried = true; return []; };
      await expect(runEasRelease(["build", "github", revision, directory], query, git)).rejects.toThrow("policy unavailable");
      expect(queried).toBe(false);
      await expect(runEasRelease(["bad"], async () => [], git)).rejects.toThrow("usage");
      delete process.env.EXPO_TOKEN;
      await expect(runEasRelease(["build", "github", revision, directory], async () => [], git)).rejects.toThrow("policy unavailable");
    } finally {
      if (oldToken === undefined) delete process.env.EXPO_TOKEN; else process.env.EXPO_TOKEN = oldToken;
      if (oldOwner === undefined) delete process.env.BEANS_EXPO_OWNER; else process.env.BEANS_EXPO_OWNER = oldOwner;
    }
    await expect(stageEasBuild({ ...build, status: "ERRORED" }, "github", revision, directory)).rejects.toThrow();
    const redirects = ["https://api.expo.dev/builds/artifact", "https://wf-artifacts.eascdn.net/build.ipa"];
    fetcher.mockImplementation(async () => redirects.length
      ? new Response(null, { status: 307, headers: { location: redirects.shift()! } }) as any
      : new Response(payload) as any);
    expect(await downloadArtifact("https://expo.dev/artifacts/build.ipa")).toEqual(new Uint8Array(payload));
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
  test("host suffix", () => { expect(hostLibrary("linux")).toBe("libbeans_mobile.so"); expect(hostLibrary("darwin")).toBe("libbeans_mobile.dylib"); expect(() => hostLibrary("win32")).toThrow(); });
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
describe("local exact-source provenance", () => {
  const zip = new Uint8Array([0x50, 0x4b, 3, 4, 1]);
  const identity = (over: object = {}) => async () => ({ identifier: "ai.amoena.beans", version: readVersion(), buildNumber: "77", ...over });
  const env = { EXPO_TOKEN: "t", BEANS_EXPO_OWNER: "o", BEANS_EXPO_PROJECT_ID: project, BEANS_APPLE_TEAM_ID: "ABCDE12345" };
  const git = (head = revision, dirty = "") => (args: string[]) => ({ exitCode: 0, stdout: Buffer.from(args.includes("rev-parse") ? head : dirty) }) as any;
  async function withEnv(values: Record<string, string | undefined>, body: () => Promise<void>) {
    const old = Object.fromEntries(Object.keys(values).map((name) => [name, process.env[name]]));
    for (const [name, value] of Object.entries(values)) { if (value === undefined) delete process.env[name]; else process.env[name] = value; }
    try { await body(); } finally { for (const [name, value] of Object.entries(old)) { if (value === undefined) delete process.env[name]; else process.env[name] = value; } }
  }
  test("runs eas --local, takes the number from the binary and refuses every mismatch", async () => {
    // Isolate build/provenance behavior below admission; this mock supplies no signing evidence.
    const admission = vi.spyOn(trust, "preflightMobileTrust").mockImplementation(() => undefined as never);
    const directory = await mkdtemp(join(tmpdir(), "beans-local-eas-"));
    try {
      await withEnv(env, async () => {
        let args: string[] = [];
        const run = (command: string[]) => { args = command; return { exitCode: 0 } as any; };
        // The fake builder writes the artifact where --output points, like eas does.
        const builder = (command: string[]) => { args = command; require("node:fs").writeFileSync(command[command.indexOf("--output") + 1], zip); return { exitCode: 0 } as any; };
        for (const [profile, platform] of [["github", "android"], ["production", "android"], ["testflight", "ios"]] as const) {
          await runLocalEasRelease([profile, revision, join(directory, profile)], identity(), builder, git());
          expect(args).toEqual(expect.arrayContaining(["--local", "--non-interactive", "--platform", platform, "--profile", profile]));
          for (const forbidden of ["--json", "--wait"]) expect(args).not.toContain(forbidden);
          const proof = JSON.parse(await readFile(join(directory, profile, `eas-${profile}.json`), "utf8"));
          expect(proof).toMatchObject({ kind: "local-exact-source", buildNumber: "77", revision, project });
          expect(proof.buildId).toBeUndefined();
          expect(JSON.stringify(proof)).not.toContain("https");
        }
        const bad = join(directory, "bad");
        for (const [name, call] of [
          ["wrong identifier", () => runLocalEasRelease(["github", revision, bad], identity({ identifier: "ai.amoena.beans.dev" }), builder, git())],
          ["wrong version", () => runLocalEasRelease(["github", revision, bad], identity({ version: "0.0.1" }), builder, git())],
          ["zero number", () => runLocalEasRelease(["github", revision, bad], identity({ buildNumber: "0" }), builder, git())],
          ["huge number", () => runLocalEasRelease(["github", revision, bad], identity({ buildNumber: "2100000001" }), builder, git())],
          ["untracked input", () => runLocalEasRelease(["github", revision, bad], identity(), builder, (args) => { if (args.includes("status")) expect(args).toContain("--untracked-files=all"); return git(revision, "?? extra.ts")(args); })],
          ["dirty tree", () => runLocalEasRelease(["github", revision, bad], identity(), builder, git(revision, "M x"))],
          ["other HEAD", () => runLocalEasRelease(["github", revision, bad], identity(), builder, git("b".repeat(40)))],
          ["build failure", () => runLocalEasRelease(["github", revision, bad], identity(), () => ({ exitCode: 1 }) as any, git())],
          ["build wrote nothing", () => runLocalEasRelease(["github", revision, bad], identity(), run, git())],
          ["not a zip", () => runLocalEasRelease(["github", revision, bad], identity(), (command) => { require("node:fs").writeFileSync(command[command.indexOf("--output") + 1], "no"); return { exitCode: 0 } as any; }, git())],
          ["existing proof", () => runLocalEasRelease(["github", revision, join(directory, "github")], identity(), builder, git())],
          ["bad profile", () => runLocalEasRelease(["preview", revision, bad], identity(), builder, git())],
        ] as const) await expect(call(), name).rejects.toThrow();
        await withEnv({ BEANS_APPLE_TEAM_ID: undefined }, async () => {
          await expect(runLocalEasRelease(["testflight", revision, bad], identity(), builder, git())).rejects.toThrow("BEANS_APPLE_TEAM_ID");
        });
        await withEnv({ EXPO_TOKEN: undefined }, async () => {
          await expect(runLocalEasRelease(["github", revision, bad], identity(), builder, git())).rejects.toThrow("required");
        });
      });
    } finally { admission.mockRestore(); await rm(directory, { recursive: true, force: true }); }
  });
});
describe("local staging binds the inspector and the staged hash to one admitted snapshot", () => {
  const zip = (tag: number) => new Uint8Array([0x50, 0x4b, 3, 4, tag]);
  const sha = (bytes: Uint8Array) => require("node:crypto").createHash("sha256").update(bytes).digest("hex");
  async function withProject(body: (directory: string) => Promise<void>) {
    const directory = await mkdtemp(join(tmpdir(), "beans-local-snapshot-"));
    const previous = process.env.BEANS_EXPO_PROJECT_ID;
    process.env.BEANS_EXPO_PROJECT_ID = project;
    try { await body(directory); } finally {
      if (previous === undefined) delete process.env.BEANS_EXPO_PROJECT_ID; else process.env.BEANS_EXPO_PROJECT_ID = previous;
      await rm(directory, { recursive: true, force: true });
    }
  }
  const ok = { identifier: "ai.amoena.beans", version: readVersion(), buildNumber: "77" };
  test("replacing the original after admission changes neither the inspected bytes nor the staged hash", async () => {
    await withProject(async (directory) => {
      const fs = require("node:fs");
      const original = join(directory, "build.apk"), admitted = zip(1), replacement = zip(2);
      fs.writeFileSync(original, admitted);
      let seen = "";
      await stageLocalBuild(original, "github", revision, join(directory, "out"), async (path) => {
        // The inspector is handed a private copy, not the original path.
        expect(path).not.toBe(original);
        expect(Buffer.from(fs.readFileSync(path))).toEqual(Buffer.from(admitted));
        // The original is swapped after admission, before staging writes anything.
        fs.writeFileSync(original, replacement);
        seen = path;
        return ok;
      });
      const name = `Beans-${readVersion()}.apk`;
      const staged = new Uint8Array(fs.readFileSync(join(directory, "out", name)));
      expect(staged).toEqual(admitted);
      const proof = JSON.parse(await readFile(join(directory, "out", "eas-github.json"), "utf8"));
      expect(proof).toMatchObject({ sha256: sha(admitted), size: admitted.length });
      expect(proof.sha256).not.toBe(sha(replacement));
      // The private snapshot is gone after staging.
      expect(fs.existsSync(seen)).toBe(false);
    });
  });
  test("a symlink input is refused before the inspector runs and nothing is staged", async () => {
    await withProject(async (directory) => {
      const fs = require("node:fs");
      const target = join(directory, "real.apk"), link = join(directory, "link.apk");
      fs.writeFileSync(target, zip(1)); fs.symlinkSync(target, link);
      let inspected = false;
      await expect(stageLocalBuild(link, "github", revision, join(directory, "out"), async () => { inspected = true; return ok; })).rejects.toThrow();
      expect(inspected).toBe(false);
      expect(fs.existsSync(join(directory, "out"))).toBe(false);
    });
  });
});

test("partial second write rolls back both owned files and permits retry", async () => {
  const dir = await mkdtemp(join(tmpdir(), "beans-pair-"));
  const entries = [{ name: "artifact", bytes: Buffer.from("binary") }, { name: "proof", bytes: Buffer.from("proof") }];
  try {
    let writes = 0;
    await expect(stagePair(dir, entries, async (file, bytes) => {
      await file.writeFile(bytes.subarray(0, 1));
      if (++writes === 2) throw new Error("injected EIO");
    })).rejects.toThrow("injected EIO");
    expect(require("node:fs").readdirSync(dir)).toEqual([]);
    await stagePair(dir, entries);
    expect(await readFile(join(dir, "artifact"), "utf8")).toBe("binary");
    expect(await readFile(join(dir, "proof"), "utf8")).toBe("proof");
    await expect(stagePair(dir, entries)).rejects.toThrow();
    expect(await readFile(join(dir, "proof"), "utf8")).toBe("proof");
  } finally { await rm(dir, { recursive: true, force: true }); }
});
test("rollback preserves a competing replacement and pre-existing proof", async () => {
  const dir = await mkdtemp(join(tmpdir(), "beans-pair-race-"));
  const fs = require("node:fs");
  try {
    await expect(stagePair(dir, [{ name: "artifact", bytes: Buffer.from("ours") }, { name: "proof", bytes: Buffer.from("ours") }], async (file, bytes) => {
      await file.writeFile(bytes);
      if (bytes.toString() === "ours") {
        fs.unlinkSync(join(dir, "artifact")); fs.writeFileSync(join(dir, "artifact"), "concurrent");
        fs.writeFileSync(join(dir, "proof"), "stale");
        throw new Error("race");
      }
    })).rejects.toThrow("race");
    expect(await readFile(join(dir, "artifact"), "utf8")).toBe("concurrent");
    expect(await readFile(join(dir, "proof"), "utf8")).toBe("stale");
  } finally { await rm(dir, { recursive: true, force: true }); }
});
test("stale proof and completed pair refuse before cloud payment or local build", async () => {
  const dir = await mkdtemp(join(tmpdir(), "beans-preflight-"));
  const old = { ...process.env };
  Object.assign(process.env, { EXPO_TOKEN: "mock", BEANS_EXPO_OWNER: "mock", BEANS_EXPO_PROJECT_ID: project });
  const git = (args: string[]) => ({ exitCode: 0, stdout: Buffer.from(args.includes("rev-parse") ? revision : "") });
  const query = vi.fn(async () => []);
  const builder = vi.fn(() => ({ exitCode: 0 }));
  const admission = vi.spyOn(trust, "preflightMobileTrust").mockImplementation(() => undefined as never);
  try {
    require("node:fs").writeFileSync(join(dir, "eas-github.json"), "stale");
    for (const complete of [false, true]) {
      if (complete) require("node:fs").writeFileSync(join(dir, `Beans-${readVersion()}.apk`), "existing");
      await expect(runEasRelease(["build", "github", revision, dir], query, git)).rejects.toThrow("already contains");
      await expect(runLocalEasRelease(["github", revision, dir], undefined, builder, git)).rejects.toThrow("already contains");
    }
    expect(query).not.toHaveBeenCalled(); expect(builder).not.toHaveBeenCalled();
    expect(await readFile(join(dir, "eas-github.json"), "utf8")).toBe("stale");
  } finally { admission.mockRestore(); process.env = old; await rm(dir, { recursive: true, force: true }); }
});

test("a concurrent proof creation rolls back only the artifact", async () => {
  const dir = await mkdtemp(join(tmpdir(), "beans-exclusive-"));
  try {
    await expect(stagePair(dir, [{ name: "artifact", bytes: Buffer.from("binary") }, { name: "proof", bytes: Buffer.from("new") }], async (file, bytes) => {
      await file.writeFile(bytes);
      require("node:fs").writeFileSync(join(dir, "proof"), "concurrent");
    })).rejects.toThrow();
    expect(require("node:fs").existsSync(join(dir, "artifact"))).toBe(false);
    expect(await readFile(join(dir, "proof"), "utf8")).toBe("concurrent");
  } finally { await rm(dir, { recursive: true, force: true }); }
});
