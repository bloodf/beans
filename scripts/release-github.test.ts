import { afterEach, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { copyFile, mkdir, mkdtemp, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { stageLocalBuild } from "./release-eas.ts";
import { buildReleaseManifest } from "./release-github.ts";
import { readVersion, ROOT } from "./app.ts";
const paths: string[] = [];
afterEach(async () => { await Promise.all(paths.map((path) => rm(path, { recursive: true, force: true }))); });
test("all-platform CLI preflight rejects missing notes before loading private credentials", async () => {
  const path = await mkdtemp(join(tmpdir(), "beans-release-preflight-")); paths.push(path);
  await mkdir(join(path, "scripts"));
  await mkdir(join(path, "updates"));
  for (const name of ["app.ts", "changelog.ts", "release-signing.ts", "release-github.ts"]) {
    await copyFile(join(ROOT, "scripts", name), join(path, "scripts", name));
  }
  await copyFile(join(ROOT, "updates", "public-key.txt"), join(path, "updates", "public-key.txt"));
  await writeFile(join(path, "package.json"), JSON.stringify({ version: "1.0.13" }, null, 2));
  await writeFile(join(path, "CHANGELOG.md"), "# Changelog\n\n## [Unreleased]\n\nPending changes.\n");
  const result = Bun.spawnSync([process.execPath, join(path, "scripts", "release-github.ts"), "check", "beans-v1.0.13", "all"], {
    cwd: path, env: { ...process.env, BEANS_UPDATE_PRIVATE_KEY: "" },
  });
  expect(result.exitCode).not.toBe(0);
  expect(result.stderr.toString()).toContain("CHANGELOG.md requires notes for 1.0.13");
});
async function server() {
  const path = await mkdtemp(join(tmpdir(), "beans-release-fixture-")); paths.push(path);
  for (const name of ["beans-server-linux-x86_64.tar.gz", "beans-server-linux-aarch64.tar.gz", "beans-server-updater.py"]) await writeFile(join(path, name), "fixture");
  return path;
}
const tag = `beans-v${readVersion()}`;
const revision = "a".repeat(40);
// Everything in an all-scope inventory except the three mobile binaries and their provenance.
async function inventory() {
  const path = await server();
  const v = readVersion();
  const names = [`Beans-${v}.zip`, `Beans-${v}.dmg`, "appcast.xml", `Beans Setup ${v}.exe`, `beans_${v}_amd64.deb`, `beans_${v}_arm64.deb`, "install-linux-amd64.sh", "install-linux-arm64.sh", ...["windows-amd64", "linux-amd64", "linux-arm64"].flatMap((p) => [`update-${p}.json`, `beans-${v}-${p}.tar.gz`])];
  for (const name of names) await writeFile(join(path, name), "fixture");
  const sha = createHash("sha256").update("fixture").digest("hex");
  for (const suffix of ["macos-aarch64.tar.gz", "linux-aarch64.tar.gz", "linux-x86_64.tar.gz", "windows-x86_64.zip"]) {
    const name = `beans-cli-${suffix}`;
    await writeFile(join(path, name), "fixture");
    await writeFile(join(path, `${name}.sha256`), `${sha}  ${name}\n`);
  }
  return path;
}
test("complete all inventory requires hash-bound EAS store provenance", async () => {
  const path = await inventory();
  const v = readVersion();
  const sha = createHash("sha256").update("fixture").digest("hex");
  for (const [index, profile] of ["github", "production", "testflight"].entries()) {
    const artifact = `Beans-${v}${profile === "github" ? ".apk" : profile === "production" ? ".aab" : "-store.ipa"}`;
    await writeFile(join(path, artifact), "fixture");
    await writeFile(join(path, `eas-${profile}.json`), JSON.stringify({ schema: 1, version: v, revision, profile, artifact, size: 7, sha256: sha, project: "11111111-1111-4111-8111-111111111111", buildId: `${index + 2}2222222-2222-4222-8222-222222222222`, buildNumber: String(40 + index), platform: profile === "testflight" ? "IOS" : "ANDROID" }));
  }
  const result = await buildReleaseManifest(tag, revision, path, "all");
  expect(result.artifacts.find((a) => a.name.endsWith("-store.ipa"))?.component).toBe("ios-store");
  expect(result.artifacts.some((a) => a.component === "ios")).toBe(false);
  expect(result).toEqual(await buildReleaseManifest(tag, revision, path, "all"));
  await writeFile(join(path, `Beans-${v}.aab`), "changed");
  await expect(buildReleaseManifest(tag, revision, path, "all")).rejects.toThrow("provenance");
});
test("local exact-source pipeline: stage with production code, finalize only through a binary verifier", async () => {
  const path = await inventory();
  const v = readVersion();
  const project = "11111111-1111-4111-8111-111111111111";
  const previous = process.env.BEANS_EXPO_PROJECT_ID;
  process.env.BEANS_EXPO_PROJECT_ID = project;
  const source = await mkdtemp(join(tmpdir(), "beans-local-source-")); paths.push(source);
  const numbers = { github: "50", production: "51", testflight: "52" } as const;
  const inspect = async (_: string, profile: "github" | "production" | "testflight") => ({ identifier: "ai.amoena.beans", version: v, buildNumber: numbers[profile] });
  try {
    for (const profile of ["github", "production", "testflight"] as const) {
      const input = join(source, profile);
      await writeFile(input, new Uint8Array([0x50, 0x4b, 3, 4, profile.length]));
      // Replacing the original after the inspector ran must not change what is staged or hashed.
      await stageLocalBuild(input, profile, revision, path, async (snapshot, kind) => {
        const result = await inspect(snapshot, kind);
        await writeFile(input, new Uint8Array([0x50, 0x4b, 3, 4, 99]));
        return result;
      });
      const staged = new Uint8Array(await Bun.file(join(path, `Beans-${v}${profile === "testflight" ? "-store.ipa" : profile === "production" ? ".aab" : ".apk"}`)).arrayBuffer());
      expect(staged).toEqual(new Uint8Array([0x50, 0x4b, 3, 4, profile.length]));
    }
  } finally { if (previous === undefined) delete process.env.BEANS_EXPO_PROJECT_ID; else process.env.BEANS_EXPO_PROJECT_ID = previous; }
  const proofFile = (profile: string) => join(path, `eas-${profile}.json`);
  const originals = Object.fromEntries(await Promise.all(["github", "production", "testflight"].map(async (profile) => [profile, await Bun.file(proofFile(profile)).json()])));
  expect(originals.github.buildId).toBeUndefined();
  // The trusted-binary gate (#275) is a required seam: no verifier, no local acceptance.
  await expect(buildReleaseManifest(tag, revision, path, "all")).rejects.toThrow("#275");
  const calls: string[] = [];
  const verify = async (_: Uint8Array, profile: string, version: string, number: string) => { calls.push(`${profile}:${version}:${number}`); };
  const manifest = await buildReleaseManifest(tag, revision, path, "all", verify);
  expect(calls).toEqual([`github:${v}:50`, `production:${v}:51`, `testflight:${v}:52`]);
  expect(manifest.artifacts.filter((a) => a.component === "provenance")).toHaveLength(3);
  await expect(buildReleaseManifest(tag, revision, path, "all", async () => { throw new Error("signer differs"); })).rejects.toThrow("signer differs");
  const refusals: [string, (proof: any) => any][] = [
    ["unknown kind", (p) => ({ ...p, kind: "cloud" })],
    ["fabricated build id", (p) => ({ ...p, buildId: "22222222-2222-4222-8222-222222222222" })],
    ["duplicate platform number", (p) => ({ ...p, buildNumber: "51" })],
    ["zero number", (p) => ({ ...p, buildNumber: "0" })],
    ["other revision", (p) => ({ ...p, revision: "b".repeat(40) })],
    ["wrong hash", (p) => ({ ...p, sha256: "0".repeat(64) })],
    ["wrong platform", (p) => ({ ...p, platform: "IOS" })],
    ["no project", (p) => ({ ...p, project: "" })],
  ];
  for (const [name, mutate] of refusals) {
    await writeFile(proofFile("github"), JSON.stringify(mutate(originals.github)));
    await expect(buildReleaseManifest(tag, revision, path, "all", verify), name).rejects.toThrow("Invalid EAS artifact provenance");
    await writeFile(proofFile("github"), JSON.stringify(originals.github));
  }
  await writeFile(join(path, `Beans-${v}.apk`), "changed");
  await expect(buildReleaseManifest(tag, revision, path, "all", verify)).rejects.toThrow("provenance");
});
test("server inventory is deterministic and cannot finalize all", async () => {
  const path = await server();
  expect(await buildReleaseManifest(tag, revision, path)).toEqual(await buildReleaseManifest(tag, revision, path));
  await expect(buildReleaseManifest(tag, revision, path, "all")).rejects.toThrow("incomplete");
});
test("tag, revision, scope and unknown assets fail closed", async () => {
  const path = await server();
  await expect(buildReleaseManifest("beans-v999.0.0", revision, path)).rejects.toThrow();
  await expect(buildReleaseManifest(tag, "HEAD", path)).rejects.toThrow();
  await expect(buildReleaseManifest(tag, revision, path, "clients" as any)).rejects.toThrow();
  await writeFile(join(path, `Beans-${readVersion()}-store.ipa`), "store");
  await expect(buildReleaseManifest(tag, revision, path)).rejects.toThrow("outside server");
});
test("missing inventory, zero files, orphan checksums and checksum mismatch fail", async () => {
  const path = await server();
  await writeFile(join(path, "beans-cli-linux-x86_64.tar.gz.sha256"), "wrong");
  await expect(buildReleaseManifest(tag, revision, path)).rejects.toThrow("no archive");
  await writeFile(join(path, "beans-cli-linux-x86_64.tar.gz"), "archive");
  await expect(buildReleaseManifest(tag, revision, path)).rejects.toThrow("differs");
  await writeFile(join(path, "beans-server-updater.py"), "");
  await expect(buildReleaseManifest(tag, revision, path)).rejects.toThrow("size");
});
