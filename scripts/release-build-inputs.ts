import { X509Certificate } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { inflateRawSync } from "node:zlib";

export function hostLibrary(platform: string): string {
  if (platform === "darwin") return "libbeans_mobile.dylib";
  if (platform === "linux") return "libbeans_mobile.so";
  throw new Error("Mobile binding generation requires macOS or Linux host");
}
export function androidNdk(env: Record<string, string | undefined>, platform: string, exists = existsSync): string {
  const explicit = env.ANDROID_NDK_HOME ?? env.ANDROID_NDK_ROOT;
  const sdk = env.ANDROID_HOME ?? env.ANDROID_SDK_ROOT ?? (platform === "darwin" ? join(homedir(), "Library/Android/sdk") : undefined);
  if (env.BEANS_ANDROID_NDK_VERSION && !/^\d+\.\d+\.\d+$/.test(env.BEANS_ANDROID_NDK_VERSION)) throw new Error("Invalid BEANS_ANDROID_NDK_VERSION");
  const path = explicit ?? (sdk && env.BEANS_ANDROID_NDK_VERSION ? join(sdk, "ndk", env.BEANS_ANDROID_NDK_VERSION) : undefined);
  if (!path || !exists(join(path, "source.properties"))) throw new Error("Set ANDROID_NDK_HOME/ANDROID_NDK_ROOT or SDK root plus BEANS_ANDROID_NDK_VERSION to an installed NDK");
  return path;
}

export type MobileReleaseProfile = "github" | "production" | "testflight";

// Public pins are independently approved inputs, never inferred from candidate bytes.
// Rotation/lineage and Apple verification are unsupported and fail closed.
export type PublicTrustPolicy =
  | { profile: "github"; signerSha256: string[]; rotation: "none" }
  | { profile: "production"; uploadSignerSha256: string; rotation: "none" }
  | { profile: "testflight"; teamId: string; certificateSha256: string[] };
export type PublicTrustPolicies = Partial<{ [P in MobileReleaseProfile]: Extract<PublicTrustPolicy, { profile: P }> }>;

function approveSigners(actual: string[], expected: string[]): void {
  if (!expected.length || expected.some(pin => !/^[a-f0-9]{64}$/.test(pin)) || new Set(expected).size !== expected.length ||
      !actual.length || new Set(actual).size !== actual.length || actual.length !== expected.length || actual.some(pin => !expected.includes(pin))) {
    throw new Error("Binary signer differs from approved public trust policy");
  }
}

// ZIP64 is deliberately unsupported: release inspection has explicit finite ceilings.
export function inspectReleaseArchive(bytes: Uint8Array): string[] {
  const data = Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const fail = () => { throw new Error("Unsafe binary archive structure or decompressed size"); };
  if (data.length < 22 || data.length > 2 ** 30) fail();
  let end = data.length - 22;
  while (end >= Math.max(0, data.length - 65557) && data.readUInt32LE(end) !== 0x06054b50) end--;
  if (end < Math.max(0, data.length - 65557) || end + 22 + data.readUInt16LE(end + 20) !== data.length) fail();
  const count = data.readUInt16LE(end + 10), size = data.readUInt32LE(end + 12), start = data.readUInt32LE(end + 16);
  if (data.readUInt32LE(end + 4) !== 0 || data.readUInt16LE(end + 8) !== count || !count || count > 20000 || start + size !== end) fail();
  let cursor = start, total = 0;
  const names = new Set<string>(), spans: [number, number][] = [];
  for (let index = 0; index < count; index++) {
    if (cursor + 46 > end || data.readUInt32LE(cursor) !== 0x02014b50) fail();
    const flags = data.readUInt16LE(cursor + 8), method = data.readUInt16LE(cursor + 10);
    const packed = data.readUInt32LE(cursor + 20), unpacked = data.readUInt32LE(cursor + 24);
    const length = data.readUInt16LE(cursor + 28), extra = data.readUInt16LE(cursor + 30), comment = data.readUInt16LE(cursor + 32);
    const local = data.readUInt32LE(cursor + 42), mode = data.readUInt32LE(cursor + 38) >>> 16;
    if (cursor + 46 + length + extra + comment > end || flags & 1 || ![0, 8].includes(method) || data.readUInt16LE(cursor + 34) || (mode & 0xf000) === 0xa000 || unpacked > 128 * 1024 * 1024 || (total += unpacked) > 512 * 1024 * 1024) fail();
    const rawName = data.subarray(cursor + 46, cursor + 46 + length), name = rawName.toString("utf8");
    if (!length || !Buffer.from(name).equals(rawName) || names.has(name) || name.startsWith("/") || /^[A-Za-z]:/.test(name) || /[\\\x00-\x1f\x7f*?\[\]]/.test(name) || name.split("/").some(part => part === ".." || part === ".")) fail();
    if (local + 30 > start || data.readUInt32LE(local) !== 0x04034b50 || data.readUInt16LE(local + 6) !== flags || data.readUInt16LE(local + 8) !== method) fail();
    const localLength = data.readUInt16LE(local + 26), localExtra = data.readUInt16LE(local + 28), payload = local + 30 + localLength + localExtra;
    if (payload + packed > start || !data.subarray(local + 30, local + 30 + localLength).equals(rawName)) fail();
    const compressed = data.subarray(payload, payload + packed);
    const actual = method === 0 ? compressed : inflateRawSync(compressed, { maxOutputLength: Math.max(1, unpacked) });
    if (actual.length !== unpacked) fail();
    names.add(name); spans.push([local, payload + packed]);
    cursor += 46 + length + extra + comment;
  }
  spans.sort((a, b) => a[0] - b[0]);
  if (cursor !== end || spans.some((span, index) => index > 0 && span[0] < spans[index - 1][1])) fail();
  return [...names];
}

// These commands inspect public artifacts only. Missing tools never downgrade verification.
function inspect(args: string[], maxBuffer = 1024 * 1024): string {
  if (!Bun.which(args[0])) throw new Error(`Binary inspection tool unavailable: ${args[0]}`);
  const result = Bun.spawnSync(args, { stdout: "pipe", stderr: "pipe", timeout: 30_000, maxBuffer });
  if (result.exitCode !== 0) throw new Error(`Binary inspection tool failed: ${args[0]}`);
  return result.stdout.toString();
}

export function inspectMobileBinary(path: string, profile: MobileReleaseProfile, version: string, number: string, policy?: PublicTrustPolicy): void {
  if (!["github", "production", "testflight"].includes(profile) || !/^\d+\.\d+\.\d+$/.test(version) || !/^[1-9]\d*$/.test(number)) throw new Error("Invalid binary inspection identity");
  const entries = inspectReleaseArchive(readFileSync(path));
  // CRC checks run only after declared and actual decompressed sizes are bounded.
  inspect(["unzip", "-tqq", path]);
  if (profile === "github") {
    if (!entries.includes("AndroidManifest.xml")) throw new Error("APK binary manifest missing");
    const badging = inspect(["aapt2", "dump", "badging", path]);
    const packages = badging.split("\n").filter(line => line.startsWith("package:"));
    const identity = packages[0]?.match(/^package: name='([^']+)' versionCode='([^']+)' versionName='([^']+)'(?:\s|$)/);
    if (packages.length !== 1 || !identity || identity[1] !== "ai.amoena.beans" || identity[2] !== number || identity[3] !== version) throw new Error("APK native identity differs from release");
    const signatures = inspect(["apksigner", "verify", "--verbose", "--print-certs", path]);
    if (!policy || policy.profile !== profile) throw new Error(`Binary approved public trust policy unavailable for ${profile}`);
    if (policy.rotation !== "none") throw new Error("APK signer rotation/lineage unsupported");
    const signers = [...signatures.matchAll(/^Signer #\d+ certificate SHA-256 digest: ([a-fA-F0-9]{64})\r?$/gm)].map(match => match[1].toLowerCase());
    approveSigners(signers, policy.signerSha256);
  } else if (profile === "production") {
    if (!entries.includes("base/manifest/AndroidManifest.xml")) throw new Error("AAB base module manifest missing");
    // bundletool decodes the actual protobuf base manifest; aapt2 does not inspect AABs.
    const values = ["/manifest/@package", "/manifest/@android:versionName", "/manifest/@android:versionCode"].map(xpath => inspect(["bundletool", "dump", "manifest", `--bundle=${path}`, "--module=base", `--xpath=${xpath}`]).trim());
    if (values[0] !== "ai.amoena.beans" || values[1] !== version || values[2] !== number) throw new Error("AAB native identity differs from release");
    const signatures = inspect(["jarsigner", "-verify", "-verbose", "-certs", path]);
    // jarsigner can exit zero for unsigned archives. It is not an approved-signer gate.
    if (!signatures.includes("jar verified.") || signatures.includes("unsigned entries")) throw new Error("AAB signature verification failed");
    if (!policy || policy.profile !== profile) throw new Error(`Binary approved public trust policy unavailable for ${profile}`);
    if (policy.rotation !== "none") throw new Error("AAB upload signer rotation unsupported");
    // keytool reads the JAR signer, not an arbitrary unsigned certificate member.
    // Multiple signers or certificate chains are unsupported rather than guessed.
    const certificates = inspect(["keytool", "-printcert", "-rfc", "-jarfile", path]).match(/-----BEGIN CERTIFICATE-----[\s\S]*?-----END CERTIFICATE-----/g) ?? [];
    approveSigners(certificates.map(pem => new X509Certificate(pem).fingerprint256.replaceAll(":", "").toLowerCase()), [policy.uploadSignerSha256]);
  } else {
    const apps = entries.filter(name => /^Payload\/[^/]+\.app\/Info\.plist$/.test(name));
    if (apps.length !== 1) throw new Error("IPA main app manifest missing or ambiguous");
    const base = apps[0].slice(0, -"Info.plist".length);
    const extensions = entries.filter(name => name.startsWith(`${base}PlugIns/`) && /^Payload\/[^/]+\.app\/PlugIns\/[^/]+\.appex\/Info\.plist$/.test(name));
    if (extensions.length !== 1) throw new Error("IPA notification module manifest missing or ambiguous");
    for (const [member, identifier] of [[apps[0], "ai.amoena.beans"], [extensions[0], "ai.amoena.beans.notify"]]) {
      const bytes = Bun.spawnSync(["unzip", "-p", path, member], { stdout: "pipe", stderr: "pipe", timeout: 30_000, maxBuffer: 1024 * 1024 });
      if (bytes.exitCode !== 0) throw new Error("IPA binary manifest read failed");
      const decoded = Bun.spawnSync(["plutil", "-convert", "json", "-o", "-", "-"], { stdin: bytes.stdout, stdout: "pipe", stderr: "pipe", timeout: 30_000, maxBuffer: 1024 * 1024 });
      if (decoded.exitCode !== 0) throw new Error("IPA binary manifest decode failed");
      const value = JSON.parse(decoded.stdout.toString());
      if (value.CFBundleIdentifier !== identifier || value.CFBundleShortVersionString !== version || value.CFBundleVersion !== number) throw new Error("IPA native identity differs from release");
    }
  }
  if (profile === "testflight") throw new Error("IPA cryptographic trust verification unsupported; all-platform publication blocked");
}
