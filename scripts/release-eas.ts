import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { lstat, open, mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { realpathSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { readVersion, ROOT } from "./app.ts";
import { inspectMobileBinary, preflightMobileTrust, publicTrustArguments, type MobileReleaseProfile, type PublicTrustPolicy } from "./release-build-inputs.ts";
import { readNativeSnapshot, snapshotArtifact } from "./release-native-identity.ts";

const profiles = { github: { platform: "ANDROID", extension: "apk", component: "android" }, production: { platform: "ANDROID", extension: "aab", component: "android-store" }, testflight: { platform: "IOS", extension: "ipa", component: "ios-store" } } as const;
type Profile = keyof typeof profiles;
function stagingNames(profile: Profile) {
  return [`Beans-${readVersion()}${profile === "testflight" ? "-store" : ""}.${profiles[profile].extension}`, `eas-${profile}.json`];
}
async function assertStageFree(directory: string, names: string[]) {
  for (const name of names) {
    try { await lstat(join(directory, name)); }
    catch (error) { if (error && typeof error === "object" && "code" in error && error.code === "ENOENT") continue; throw error; }
    throw new Error(`Release staging already contains ${name}`);
  }
}
// Retain open handles through rollback so partial writes are owned before writing starts.
export async function stagePair(directory: string, entries: { name: string; bytes: Uint8Array }[], write = (file: Awaited<ReturnType<typeof open>>, bytes: Uint8Array) => file.writeFile(bytes)) {
  await mkdir(directory, { recursive: true });
  const created: { path: string; file: Awaited<ReturnType<typeof open>>; dev: number; ino: number }[] = [];
  try {
    for (const entry of entries) {
      const path = join(directory, entry.name);
      const file = await open(path, "wx", 0o600);
      const owned = { path, file, dev: -1, ino: -1 };
      created.push(owned);
      const identity = await file.stat();
      owned.dev = identity.dev; owned.ino = identity.ino;
      await write(file, entry.bytes);
    }
  } catch (error) {
    for (const owned of created.reverse()) {
      try {
        const current = await lstat(owned.path);
        if (current.dev === owned.dev && current.ino === owned.ino) await rm(owned.path);
      } catch (cleanup) {
        if (!(cleanup && typeof cleanup === "object" && "code" in cleanup && cleanup.code === "ENOENT")) throw new AggregateError([error, cleanup], "Staging failed and rollback requires inspection");
      }
    }
    throw error;
  } finally { await Promise.all(created.map(({ file }) => file.close())); }
}
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
  const child = spawn("eas", args, { cwd: join(ROOT, "mobile"), stdio: ["ignore", "pipe", "inherit"] });
  const chunks: Buffer[] = [];
  child.stdout!.on("data", (chunk: Buffer) => chunks.push(chunk));
  const [code, signal] = await new Promise<[number | null, NodeJS.Signals | null]>((settle, reject) => {
    child.once("error", reject);
    child.once("close", (exitCode, exitSignal) => settle([exitCode, exitSignal]));
  });
  const text = Buffer.concat(chunks).toString("utf8");
  if (code === null) throw new Error(`EAS command failed with signal ${signal ?? "unknown"}; no release readiness is published`);
  if (code !== 0) throw new Error("EAS command failed; no release readiness is published");
  return JSON.parse(text);
}
export async function verifyEasBinary(bytes: Uint8Array, profile: MobileReleaseProfile, version: string, number: string, policy?: PublicTrustPolicy) {
  if (!Object.hasOwn(profiles, profile)) throw new Error("Invalid binary inspection profile");
  const approved = preflightMobileTrust(profile, policy);
  const temporary = await mkdtemp(join(tmpdir(), "beans-binary-inspect-"));
  try {
    const path = join(temporary, `candidate.${profiles[profile].extension}`);
    await writeFile(path, bytes, { flag: "wx" });
    inspectMobileBinary(path, profile, version, number, approved);
  } finally { await rm(temporary, { recursive: true, force: true }); }
}
export async function stageEasBuild(build: any, profile: Profile, revision: string, directory: string, policy?: PublicTrustPolicy) {
  const project = process.env.BEANS_EXPO_PROJECT_ID ?? "";
  validateEasBuild(build, profile, revision, readVersion(), project);
  const approved = preflightMobileTrust(profile, policy);
  await assertStageFree(directory, stagingNames(profile));
  const bytes = await downloadArtifact(build.artifacts.buildUrl);
  await verifyEasBinary(bytes, profile, readVersion(), build.appBuildVersion, approved);
  const name = `Beans-${readVersion()}${profile === "testflight" ? "-store" : ""}.${profiles[profile].extension}`;
  // URLs can carry download tokens; provenance records identifiers and hashes, never URLs.
  const provenance = { schema: 1, revision, version: readVersion(), project, buildId: build.id, buildNumber: build.appBuildVersion, profile, platform: build.platform, artifact: name, size: bytes.length, sha256: createHash("sha256").update(bytes).digest("hex") };
  await stagePair(directory, [{ name, bytes }, { name: `eas-${profile}.json`, bytes: Buffer.from(JSON.stringify(provenance) + "\n") }]);
}
// Local exact-source builds (`eas build --local`) still draw their number from EAS remote versions,
// but EAS records no build for them: the proof has no buildId and the number is read from the binary.
export type NativeIdentity = { identifier: string; version: string; buildNumber: string };
export type InspectNative = (path: string, profile: Profile) => Promise<NativeIdentity>;
export async function stageLocalBuild(input: string, profile: Profile, revision: string, directory: string, inspect?: InspectNative) {
  const project = process.env.BEANS_EXPO_PROJECT_ID ?? "";
  const version = readVersion();
  if (!Object.hasOwn(profiles, profile) || !uuid.test(project) || !/^[a-f0-9]{40}$/.test(revision)) throw new Error("Invalid EAS release identity");
  // One admitted snapshot: the inspector sees the private copy, and the artifact and proof hash are
  // written from the same retained bytes, so replacing `input` afterwards changes neither.
  await assertStageFree(directory, stagingNames(profile));
  const snapshot = snapshotArtifact(input);
  try {
    const bytes = snapshot.bytes;
    if (bytes.length < 4 || bytes[0] !== 0x50 || bytes[1] !== 0x4b) throw new Error("Local artifact must be a nonempty ZIP-format APK/AAB/IPA");
    const native = await (inspect ? inspect(snapshot.path, profile) : readNativeSnapshot(snapshot, profile));
    if (native?.identifier !== "ai.amoena.beans" || native.version !== version || !/^[1-9]\d*$/.test(native.buildNumber) || Number(native.buildNumber) > 2_100_000_000) {
      throw new Error("Local binary differs from the release identity or version, or its build number is not a positive integer at most 2100000000");
    }
    const name = `Beans-${version}${profile === "testflight" ? "-store" : ""}.${profiles[profile].extension}`;
    const provenance = { schema: 1, kind: "local-exact-source", revision, version, project, profile, platform: profiles[profile].platform, buildNumber: native.buildNumber, artifact: name, size: bytes.length, sha256: createHash("sha256").update(bytes).digest("hex") };
    await stagePair(directory, [{ name, bytes }, { name: `eas-${profile}.json`, bytes: Buffer.from(JSON.stringify(provenance) + "\n") }]);
  } finally { snapshot.dispose(); }
}
export async function runLocalEasRelease(args: string[], inspect: InspectNative | undefined = undefined, run = (args: string[], cwd: string) => ({ exitCode: spawnSync(args[0]!, args.slice(1), { cwd, stdio: "inherit" }).status }), git = gitState) {
  const parsed = await publicTrustArguments(args);
  const [profile, revision, directory, ...extra] = parsed.args;
  if (!Object.hasOwn(profiles, profile) || !revision || !directory || extra.length) throw new Error("usage: <github|production|testflight> <revision> <directory>");
  preflightMobileTrust(profile as Profile, parsed.policies[profile as Profile]);
  if (!process.env.EXPO_TOKEN || !process.env.BEANS_EXPO_OWNER || !uuid.test(process.env.BEANS_EXPO_PROJECT_ID ?? "")) throw new Error("EXPO_TOKEN, BEANS_EXPO_OWNER and linked BEANS_EXPO_PROJECT_ID are required");
  if (profile === "testflight" && !/^[A-Z0-9]{10}$/.test(process.env.BEANS_APPLE_TEAM_ID ?? "")) throw new Error("BEANS_APPLE_TEAM_ID is required for a local store IPA");
  const head = git(["git", "rev-parse", "HEAD"]);
  const dirty = git(["git", "status", "--porcelain", "--untracked-files=all"]);
  if (head.exitCode || dirty.exitCode || head.stdout.toString().trim() !== revision || dirty.stdout.length) throw new Error("EAS release requires clean exact-revision checkout");
  const typedProfile = profile as Profile;
  await assertStageFree(resolve(directory), stagingNames(typedProfile));
  const scratch = await mkdtemp(join(tmpdir(), "beans-local-build-"));
  try {
    const output = join(scratch, `build.${profiles[typedProfile].extension}`);
    // No BUILD_NUMBER, no --json/--wait: EAS stays the build-number authority and no build record exists.
    const built = run(["eas", "build", "--local", "--platform", profiles[typedProfile].platform.toLowerCase(), "--profile", profile, "--non-interactive", "--output", output], join(ROOT, "mobile"));
    if (built.exitCode !== 0) throw new Error("Local EAS build failed; no release readiness is published");
    await stageLocalBuild(output, typedProfile, revision, resolve(directory), inspect);
  } finally { await rm(scratch, { recursive: true, force: true }); }
}
/** The release gate reads the checkout state. A spawn failure or a signalled git is a failure,
 * never a clean tree. */
function gitState(args: string[]): { exitCode: number; stdout: Buffer } {
  const result = spawnSync(args[0]!, args.slice(1), { cwd: ROOT });
  if (result.error || result.status === null) return { exitCode: 1, stdout: Buffer.alloc(0) };
  return { exitCode: result.status, stdout: result.stdout };
}
export async function runEasRelease(args: string[], query = eas, git = gitState) {
  const parsed = await publicTrustArguments(args);
  const [command, profile, revision, directory, buildId, ...extra] = parsed.args;
  if (command !== "build" || !Object.hasOwn(profiles, profile) || !revision || !directory || extra.length) throw new Error("usage: release-eas.ts build <github|production|testflight> <revision> <directory> [retry-build-id]");
  const policy = preflightMobileTrust(profile as Profile, parsed.policies[profile as Profile]);
  if (!process.env.EXPO_TOKEN || !process.env.BEANS_EXPO_OWNER || !uuid.test(process.env.BEANS_EXPO_PROJECT_ID ?? "")) throw new Error("EXPO_TOKEN, BEANS_EXPO_OWNER and linked BEANS_EXPO_PROJECT_ID are required");
  const head = git(["git", "rev-parse", "HEAD"]);
  const dirty = git(["git", "status", "--porcelain", "--untracked-files=all"]);
  if (head.exitCode || dirty.exitCode || head.stdout.toString().trim() !== revision || dirty.stdout.length) throw new Error("EAS release requires clean exact-revision checkout");
  const typedProfile = profile as Profile;
  await assertStageFree(resolve(directory), stagingNames(typedProfile));
  const builds = buildId ? [await query(["build:view", buildId, "--json"])] : await query(["build", "--platform", profiles[typedProfile].platform.toLowerCase(), "--profile", profile, "--non-interactive", "--wait", "--json"]);
  if (!Array.isArray(builds) || builds.length !== 1) throw new Error("Expected exactly one completed EAS build");
  await stageEasBuild(builds[0], typedProfile, revision, resolve(directory), policy);
}
/** Dispatch only when this file is the process entry; importing it must stay inert. */
const isMain = (() => {
  const meta = import.meta as ImportMeta & { main?: boolean };
  if (meta.main === true) return true;
  const entry = process.argv[1];
  if (!entry) return false;
  const canonical = (path: string) => { try { return realpathSync(path); } catch { return resolve(path); } };
  try {
    return canonical(fileURLToPath(meta.url)) === canonical(resolve(entry));
  } catch {
    return false;
  }
})();

if (isMain) await runEasRelease(process.argv.slice(2));
