import { closeSync, constants, fstatSync, mkdtempSync, openSync, readSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { inspectReleaseArchive } from "./release-build-inputs.ts";
import type { InspectNative, NativeIdentity } from "./release-eas.ts";

// Public native metadata only: identifier, version and build number printed by installed tools.
// This is NOT a trust decision. Signer, team and entitlement verification stay a separate mandatory
// gate (the verifier buildReleaseManifest requires), and archive shape here proves nothing about origin.
type Profile = Parameters<InspectNative>[1];
type Env = Record<string, string | undefined>;

// Argument vectors only, finite time and output, isolated locale. A missing tool or nonzero exit refuses.
function run(args: string[], env: Env, input?: Uint8Array): Buffer {
  if (!Bun.which(args[0], { PATH: env.PATH ?? "" })) throw new Error(`Binary inspection tool unavailable: ${args[0]}`);
  const result = Bun.spawnSync(args, { env: { ...env, LC_ALL: "C", LANG: "C" }, stdin: input, stdout: "pipe", stderr: "pipe", timeout: 30_000, maxBuffer: 1024 * 1024 });
  if (result.exitCode !== 0) throw new Error(`Binary inspection tool failed: ${args[0]}`);
  return result.stdout;
}
const field = (value: unknown): string => {
  if (typeof value !== "string" || !value || value.length > 256 || /[\x00-\x1f\x7f]/.test(value)) throw new Error("Native identity field is missing or malformed");
  return value;
};
const MAX_ARTIFACT = 2 ** 30;
export type ArtifactSnapshot = { bytes: Buffer; path: string; dispose: () => void };
// One admission of one byte snapshot. The input is opened once without following a final symlink, must be a
// regular bounded file, and is read once with the read capped at fstat size + 1 so growth while reading is
// detected instead of buffered. Native tools and release staging then use only these bytes (the private
// 0700-directory / 0600 copy and the retained buffer), never the original path again.
export function snapshotArtifact(input: string): ArtifactSnapshot {
  if (!input.startsWith("/")) throw new Error("Native artifact path must be absolute");
  // O_NONBLOCK keeps open() from waiting forever on a FIFO with no writer; the fstat check below then refuses
  // anything that is not a regular file, and a regular file is unaffected by the flag.
  const fd = openSync(input, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
  let bytes: Buffer;
  try {
    const info = fstatSync(fd);
    if (!info.isFile() || info.size < 1 || info.size > MAX_ARTIFACT) throw new Error("Native artifact must be a regular unlinked file of bounded size");
    bytes = Buffer.allocUnsafe(info.size + 1);
    let length = 0;
    for (;;) {
      const read = readSync(fd, bytes, length, bytes.length - length, null);
      if (read === 0) break;
      length += read;
      if (length === bytes.length) break;
    }
    if (length !== info.size) throw new Error("Native artifact changed while it was read");
    bytes = bytes.subarray(0, length);
  } finally { closeSync(fd); }
  const directory = mkdtempSync(join(tmpdir(), "beans-native-snapshot-"));
  const dispose = () => rmSync(directory, { recursive: true, force: true });
  try {
    const path = join(directory, "artifact");
    writeFileSync(path, bytes, { flag: "wx", mode: 0o600 });
    return { bytes, path, dispose };
  } catch (error) { dispose(); throw error; }
}

export function readNativeIdentity(path: string, profile: Profile, env: Env = process.env): NativeIdentity {
  if (!["github", "production", "testflight"].includes(profile) || !path.startsWith("/")) throw new Error("Invalid native identity request");
  const snapshot = snapshotArtifact(path);
  try { return identityOf(snapshot, profile, env); } finally { snapshot.dispose(); }
}
function identityOf({ bytes, path }: ArtifactSnapshot, profile: Profile, env: Env): NativeIdentity {
  // Hostile archive structure is refused by the shared bounded inspector before any tool runs; tools read
  // the private snapshot path only.
  const names = inspectReleaseArchive(bytes);
  run(["unzip", "-tqq", path], env);
  if (profile === "github") {
    if (!names.includes("AndroidManifest.xml")) throw new Error("APK binary manifest missing");
    const packages = run(["aapt2", "dump", "badging", path], env).toString().split("\n").filter(line => line.startsWith("package:"));
    const match = packages[0]?.match(/^package: name='([^']+)' versionCode='([^']+)' versionName='([^']+)'(?:\s|$)/);
    if (packages.length !== 1 || !match) throw new Error("APK native identity is missing or ambiguous");
    return { identifier: field(match[1]), buildNumber: field(match[2]), version: field(match[3]) };
  }
  if (profile === "production") {
    if (!names.includes("base/manifest/AndroidManifest.xml")) throw new Error("AAB base module manifest missing");
    // bundletool decodes the protobuf base manifest; aapt2 cannot read AABs.
    const [identifier, version, buildNumber] = ["/manifest/@package", "/manifest/@android:versionName", "/manifest/@android:versionCode"]
      .map(xpath => field(run(["bundletool", "dump", "manifest", `--bundle=${path}`, "--module=base", `--xpath=${xpath}`], env).toString().trim()));
    return { identifier, version, buildNumber };
  }
  const apps = names.filter(name => /^Payload\/[^/]+\.app\/Info\.plist$/.test(name));
  if (apps.length !== 1) throw new Error("IPA main app manifest missing or ambiguous");
  const base = apps[0].slice(0, -"Info.plist".length);
  const extensions = names.filter(name => name.startsWith(`${base}PlugIns/`) && /^Payload\/[^/]+\.app\/PlugIns\/[^/]+\.appex\/Info\.plist$/.test(name));
  if (extensions.length !== 1) throw new Error("IPA notification module manifest missing or ambiguous");
  const plist = (member: string) => {
    const value = JSON.parse(run(["plutil", "-convert", "json", "-o", "-", "-"], env, run(["unzip", "-p", path, member], env)).toString());
    return { identifier: field(value.CFBundleIdentifier), version: field(value.CFBundleShortVersionString), buildNumber: field(value.CFBundleVersion) };
  };
  const app = plist(apps[0]), extension = plist(extensions[0]);
  if (extension.identifier !== "ai.amoena.beans.notify" || extension.version !== app.version || extension.buildNumber !== app.buildNumber) {
    throw new Error("IPA notification extension differs from the app");
  }
  return app;
}
