import { afterEach, expect, test } from "bun:test";
import { copyFile, mkdir, mkdtemp, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { buildReleaseManifest } from "./release-github.ts";
import { readVersion, ROOT } from "./app.ts";
const paths: string[] = [];
afterEach(async () => { await Promise.all(paths.map((path) => rm(path, { recursive: true, force: true }))); });
test("all-platform CLI refuses absent or invalid public trust before changelog and key access", async () => {
  const path = await mkdtemp(join(tmpdir(), "beans-release-preflight-")); paths.push(path);
  await mkdir(join(path, "scripts"));
  await mkdir(join(path, "updates"));
  for (const name of ["app.ts", "changelog.ts", "release-signing.ts", "release-github.ts", "release-eas.ts", "release-build-inputs.ts", "release-ipa-claims.ts"]) {
    await copyFile(join(ROOT, "scripts", name), join(path, "scripts", name));
  }
  await copyFile(join(ROOT, "updates", "public-key.txt"), join(path, "updates", "public-key.txt"));
  await writeFile(join(path, "package.json"), JSON.stringify({ version: "1.0.13" }, null, 2));
  await writeFile(join(path, "CHANGELOG.md"), "# Changelog\n\n## [Unreleased]\n\nPending changes.\n");
  const check = (args: string[]) => Bun.spawnSync([process.execPath, join(path, "scripts", "release-github.ts"), "check", "beans-v1.0.13", "all", ...args], {
    cwd: path, env: { ...process.env, BEANS_UPDATE_PRIVATE_KEY: "" },
  });
  const unconfigured = check([]);
  expect(unconfigured.exitCode).not.toBe(0);
  expect(unconfigured.stderr.toString()).toContain("trust policy unavailable for github");
  expect(unconfigured.stderr.toString()).not.toContain("CHANGELOG.md requires notes");
  await writeFile(join(path, "invalid-policy.json"), JSON.stringify({ github: { profile: "github", rotation: "later", signerSha256: [] } }));
  const configured = check(["--public-trust-policy", join(path, "invalid-policy.json")]);
  expect(configured.exitCode).not.toBe(0);
  expect(configured.stderr.toString()).toContain("Invalid APK current-signer policy");
  expect(configured.stderr.toString()).not.toContain("CHANGELOG.md requires notes");
});
async function server() {
  const path = await mkdtemp(join(tmpdir(), "beans-release-fixture-")); paths.push(path);
  for (const name of ["beans-server-linux-x86_64.tar.gz", "beans-server-linux-aarch64.tar.gz", "beans-server-updater.py"]) await writeFile(join(path, name), "fixture");
  return path;
}
const tag = `beans-v${readVersion()}`;
const revision = "a".repeat(40);
test("all-platform finalization cannot inspect or sign inventory without public authority", async () => {
  const path = await server();
  await expect(buildReleaseManifest(tag, revision, path, "all")).rejects.toThrow("trust policy unavailable for github");
});
test("server inventory is deterministic and cannot finalize all", async () => {
  const path = await server();
  expect(await buildReleaseManifest(tag, revision, path)).toEqual(await buildReleaseManifest(tag, revision, path));
  await expect(buildReleaseManifest(tag, revision, path, "all")).rejects.toThrow("policy unavailable");
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
