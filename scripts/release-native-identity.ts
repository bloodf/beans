import { lstatSync, readFileSync } from "node:fs";
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
export function readNativeIdentity(path: string, profile: Profile, env: Env = process.env): NativeIdentity {
  if (!["github", "production", "testflight"].includes(profile) || !path.startsWith("/")) throw new Error("Invalid native identity request");
  const info = lstatSync(path);
  if (!info.isFile() || info.size < 22 || info.size > 2 ** 30) throw new Error("Native artifact must be a regular unlinked file of bounded size");
  // Hostile archive structure is refused by the shared bounded inspector before any tool runs.
  const names = inspectReleaseArchive(readFileSync(path));
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
