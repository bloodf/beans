import { createHash } from "node:crypto";
import { mkdir, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { readVersion, ROOT } from "./app.ts";

const profiles = { github: { platform: "ANDROID", extension: "apk", component: "android" }, production: { platform: "ANDROID", extension: "aab", component: "android-store" }, testflight: { platform: "IOS", extension: "ipa", component: "ios-store" } } as const;
type Profile = keyof typeof profiles;
const uuid = /^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/i;
export function validateEasBuild(value: any, profile: Profile, revision: string, version: string, project: string) {
  if (!profiles[profile] || !uuid.test(project) || !/^[a-f0-9]{40}$/.test(revision)) throw new Error("Invalid EAS release identity");
  if (!value || !uuid.test(value.id) || value.status !== "FINISHED" || value.platform !== profiles[profile].platform ||
      value.buildProfile !== profile || value.gitCommitHash !== revision || value.appVersion !== version || value.app?.id !== project ||
      value.appIdentifier !== "ai.amoena.beans" || value.distribution !== "STORE" || value.isForIosSimulator === true ||
      !/^[1-9]\d*$/.test(value.appBuildVersion) || Number(value.appBuildVersion) > 2_100_000_000) {
    throw new Error("EAS build is unfinished or differs from the exact release source, profile, project or version");
  }
  validateArtifactUrl(value.artifacts?.buildUrl);
  return value;
}
export function validateArtifactUrl(value: string): URL {
  const url = new URL(value);
  if (url.protocol !== "https:" || url.username || url.password || !["artifacts.eascdn.net", "wf-artifacts.eascdn.net", "api.expo.dev", "expo.dev", "exp.host", "storage.googleapis.com"].includes(url.hostname)) throw new Error("EAS artifact URL must use a trusted HTTPS artifact host");
  return url;
}
export async function downloadArtifact(value: string): Promise<Uint8Array> {
  let url = validateArtifactUrl(value);
  let response: Response | undefined;
  for (let redirects = 0; redirects <= 5; redirects++) {
    response = await fetch(url, { redirect: "manual", signal: AbortSignal.timeout(120_000) });
    if (![301, 302, 303, 307, 308].includes(response.status)) break;
    const location = response.headers.get("location");
    await response.body?.cancel();
    if (!location || redirects === 5) throw new Error("Invalid EAS artifact redirect");
    url = validateArtifactUrl(new URL(location, url).href);
  }
  if (!response?.ok || !response.body) throw new Error(`EAS artifact download failed (${response?.status})`);
  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let size = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.length;
      if (size > 2 ** 30) throw new Error("EAS artifact exceeds 1 GiB");
      chunks.push(value);
    }
  } finally { await reader.cancel(); }
  const bytes = new Uint8Array(Buffer.concat(chunks));
  if (bytes.length < 4 || bytes[0] !== 0x50 || bytes[1] !== 0x4b) throw new Error("EAS artifact must be a nonempty ZIP-format APK/AAB/IPA");
  return bytes;
}
async function eas(args: string[]): Promise<any> {
  const child = Bun.spawn(["eas", ...args], { cwd: join(ROOT, "mobile"), stdout: "pipe", stderr: "inherit" });
  const text = await new Response(child.stdout).text();
  if (await child.exited !== 0) throw new Error("EAS command failed; no release readiness is published");
  return JSON.parse(text);
}
export async function stageEasBuild(build: any, profile: Profile, revision: string, directory: string) {
  const project = process.env.BEANS_EXPO_PROJECT_ID ?? "";
  validateEasBuild(build, profile, revision, readVersion(), project);
  const bytes = await downloadArtifact(build.artifacts.buildUrl);
  const name = `Beans-${readVersion()}${profile === "testflight" ? "-store" : ""}.${profiles[profile].extension}`;
  await mkdir(directory, { recursive: true });
  await writeFile(join(directory, name), bytes, { flag: "wx" });
  // URLs can carry download tokens; provenance records identifiers and hashes, never URLs.
  const provenance = { schema: 1, revision, version: readVersion(), project, buildId: build.id, buildNumber: build.appBuildVersion, profile, platform: build.platform, artifact: name, size: bytes.length, sha256: createHash("sha256").update(bytes).digest("hex") };
  await writeFile(join(directory, `eas-${profile}.json`), `${JSON.stringify(provenance)}\n`, { flag: "wx" });
}
export async function runEasRelease(args: string[], query = eas, git = (args: string[]) => Bun.spawnSync(args, { cwd: ROOT })) {
  const [command, profile, revision, directory, buildId, ...extra] = args;
  if (command !== "build" || !(profile in profiles) || !revision || !directory || extra.length) throw new Error("usage: release-eas.ts build <github|production|testflight> <revision> <directory> [retry-build-id]");
  if (!process.env.EXPO_TOKEN || !process.env.BEANS_EXPO_OWNER || !uuid.test(process.env.BEANS_EXPO_PROJECT_ID ?? "")) throw new Error("EXPO_TOKEN, BEANS_EXPO_OWNER and linked BEANS_EXPO_PROJECT_ID are required");
  const head = git(["git", "rev-parse", "HEAD"]);
  const dirty = git(["git", "status", "--porcelain", "--untracked-files=no"]);
  if (head.exitCode || dirty.exitCode || head.stdout.toString().trim() !== revision || dirty.stdout.length) throw new Error("EAS release requires clean exact-revision checkout");
  const typedProfile = profile as Profile;
  const builds = buildId ? [await query(["build:view", buildId, "--json"])] : await query(["build", "--platform", profiles[typedProfile].platform.toLowerCase(), "--profile", profile, "--non-interactive", "--wait", "--json"]);
  if (!Array.isArray(builds) || builds.length !== 1) throw new Error("Expected exactly one completed EAS build");
  await stageEasBuild(builds[0], typedProfile, revision, resolve(directory));
}
if (import.meta.main) await runEasRelease(process.argv.slice(2));
