import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

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

// These commands inspect public artifacts only. Missing tools never downgrade verification.
function inspect(args: string[], maxBuffer = 1024 * 1024): string {
  if (!Bun.which(args[0])) throw new Error(`Binary inspection tool unavailable: ${args[0]}`);
  const result = Bun.spawnSync(args, { stdout: "pipe", stderr: "pipe", timeout: 30_000, maxBuffer });
  if (result.exitCode !== 0) throw new Error(`Binary inspection tool failed: ${args[0]}`);
  return result.stdout.toString();
}

export function inspectMobileBinary(path: string, profile: MobileReleaseProfile, version: string, number: string): void {
  if (!["github", "production", "testflight"].includes(profile) || !/^\d+\.\d+\.\d+$/.test(version) || !/^[1-9]\d*$/.test(number)) throw new Error("Invalid binary inspection identity");
  const listing = inspect(["unzip", "-Z1", path]);
  const entries = listing.trimEnd().split("\n");
  if (!entries.length || entries.length > 20_000 || new Set(entries).size !== entries.length || entries.some(name => !name || name.startsWith("/") || name.includes("\\") || name.split("/").includes("..") || /[\x00-\x1f\x7f]/.test(name))) throw new Error("Unsafe binary archive module inventory");
  // Test archive CRCs without extracting untrusted paths.
  inspect(["unzip", "-tqq", path]);
  if (profile === "github") {
    if (!entries.includes("AndroidManifest.xml")) throw new Error("APK binary manifest missing");
    const badging = inspect(["aapt2", "dump", "badging", path]);
    const packages = badging.split("\n").filter(line => line.startsWith("package:"));
    const identity = packages[0]?.match(/^package: name='([^']+)' versionCode='([^']+)' versionName='([^']+)'(?:\s|$)/);
    if (packages.length !== 1 || !identity || identity[1] !== "ai.amoena.beans" || identity[2] !== number || identity[3] !== version) throw new Error("APK native identity differs from release");
    inspect(["apksigner", "verify", "--verbose", "--print-certs", path]);
  } else if (profile === "production") {
    if (!entries.includes("base/manifest/AndroidManifest.xml")) throw new Error("AAB base module manifest missing");
    // bundletool decodes the actual protobuf base manifest; aapt2 does not inspect AABs.
    const values = ["/manifest/@package", "/manifest/@android:versionName", "/manifest/@android:versionCode"].map(xpath => inspect(["bundletool", "dump", "manifest", `--bundle=${path}`, "--module=base", `--xpath=${xpath}`]).trim());
    if (values[0] !== "ai.amoena.beans" || values[1] !== version || values[2] !== number) throw new Error("AAB native identity differs from release");
    const signatures = inspect(["jarsigner", "-verify", "-verbose", "-certs", path]);
    // jarsigner can exit zero for unsigned archives. It is not an approved-signer gate.
    if (!signatures.includes("jar verified.") || signatures.includes("unsigned entries")) throw new Error("AAB signature verification failed");
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
  // A candidate's certificate cannot bootstrap trust. No approved policy is shipped yet.
  // Apple additionally needs signed entitlements, chain and provisioning verification.
  throw new Error(`Binary approved public signer/lineage policy unavailable for ${profile}`);
}
