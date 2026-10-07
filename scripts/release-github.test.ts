import { afterEach, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { copyFile, mkdir, mkdtemp, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
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
test("complete all inventory requires hash-bound EAS store provenance", async () => {
  const path = await server();
  const v = readVersion();
  const names = [`Beans-${v}.zip`, `Beans-${v}.dmg`, "appcast.xml", `Beans-${v}.apk`, `Beans-${v}.aab`, `Beans-${v}-store.ipa`, `Beans Setup ${v}.exe`, `beans_${v}_amd64.deb`, `beans_${v}_arm64.deb`, "install-linux-amd64.sh", "install-linux-arm64.sh", ...["windows-amd64", "linux-amd64", "linux-arm64"].flatMap((p) => [`update-${p}.json`, `beans-${v}-${p}.tar.gz`])];
  for (const name of names) await writeFile(join(path, name), "fixture");
  const sha = createHash("sha256").update("fixture").digest("hex");
  for (const suffix of ["macos-aarch64.tar.gz", "linux-aarch64.tar.gz", "linux-x86_64.tar.gz", "windows-x86_64.zip"]) {
    const name = `beans-cli-${suffix}`;
    await writeFile(join(path, name), "fixture");
    await writeFile(join(path, `${name}.sha256`), `${sha}  ${name}\n`);
  }
  for (const [index, profile] of ["github", "production", "testflight"].entries()) {
    const artifact = `Beans-${v}${profile === "github" ? ".apk" : profile === "production" ? ".aab" : "-store.ipa"}`;
    await writeFile(join(path, `eas-${profile}.json`), JSON.stringify({ schema: 1, version: v, revision, profile, artifact, size: 7, sha256: sha, project: "11111111-1111-4111-8111-111111111111", buildId: `${index + 2}2222222-2222-4222-8222-222222222222`, buildNumber: String(40 + index), platform: profile === "testflight" ? "IOS" : "ANDROID" }));
  }
  const result = await buildReleaseManifest(tag, revision, path, "all");
  expect(result.artifacts.find((a) => a.name.endsWith("-store.ipa"))?.component).toBe("ios-store");
  expect(result.artifacts.some((a) => a.component === "ios")).toBe(false);
  expect(result).toEqual(await buildReleaseManifest(tag, revision, path, "all"));
  await writeFile(join(path, `Beans-${v}.aab`), "changed");
  await expect(buildReleaseManifest(tag, revision, path, "all")).rejects.toThrow("provenance");
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
