// Public trust for mobile release binaries (#275). A candidate's own certificate can never bootstrap
// trust: every profile needs an explicitly approved, public policy of pinned signer digests, supplied
// with `--public-trust-policy <file>`. Admission checks the exact current signer set. Apple needs
// authoritative provisioning and team verification that does not exist yet, so IPA staging and
// all-platform publication fail closed even with a policy.
import { spawnSync } from "node:child_process";
import { X509Certificate } from "node:crypto";
import { existsSync, mkdtempSync, readFileSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import { inspectIpaSignedClaims } from "./release-ipa-claims.ts";
import { readNativeSnapshot, snapshotBytes, type ArtifactSnapshot } from "./release-native-identity.ts";

export type MobileReleaseProfile = "github" | "production" | "testflight";
export const MOBILE_PROFILES = ["github", "production", "testflight"] as const;

// Public pins are independently approved inputs, never inferred from candidate bytes.
export type PublicTrustPolicy =
  | { profile: "github"; signerSha256: string[]; rotation: "none" }
  | { profile: "production"; uploadSignerSha256: string; rotation: "none" }
  | { profile: "testflight"; teamId: string; certificateSha256: string[] };
export type PublicTrustPolicies = Partial<{ [P in MobileReleaseProfile]: Extract<PublicTrustPolicy, { profile: P }> }>;
type Env = Record<string, string | undefined>;

export const IPA_UNSUPPORTED = "IPA cryptographic trust verification unsupported; all-platform publication blocked";
const MAX_POLICY_BYTES = 64 * 1024;

/** The exact signer set must equal the approved pins: no extra, no missing, no duplicate signer. */
export function approveSigners(actual: string[], expected: string[]): void {
  if (!expected.length || expected.some(pin => typeof pin !== "string" || !/^[a-f0-9]{64}$/.test(pin)) || new Set(expected).size !== expected.length ||
      !actual.length || new Set(actual).size !== actual.length || actual.length !== expected.length || actual.some(pin => !expected.includes(pin))) {
    throw new Error("Binary signer differs from approved public trust policy");
  }
}

export function validatePublicTrustPolicy(value: unknown, profile: MobileReleaseProfile): PublicTrustPolicy {
  const policy = value as PublicTrustPolicy;
  if (!policy || typeof policy !== "object" || Array.isArray(policy) || policy.profile !== profile) throw new Error(`Binary approved public trust policy unavailable for ${profile}`);
  const keys = Object.keys(policy).sort().join(",");
  if (policy.profile === "github") {
    if (keys !== "profile,rotation,signerSha256" || policy.rotation !== "none" || !Array.isArray(policy.signerSha256)) throw new Error("Invalid APK current-signer policy");
    approveSigners(policy.signerSha256, policy.signerSha256);
  } else if (policy.profile === "production") {
    if (keys !== "profile,rotation,uploadSignerSha256" || policy.rotation !== "none") throw new Error("Invalid AAB upload-signer policy");
    approveSigners([policy.uploadSignerSha256], [policy.uploadSignerSha256]);
  } else {
    if (keys !== "certificateSha256,profile,teamId" || typeof policy.teamId !== "string" || !/^[A-Z0-9]{10}$/.test(policy.teamId) || !Array.isArray(policy.certificateSha256)) throw new Error("Invalid IPA public policy");
    approveSigners(policy.certificateSha256, policy.certificateSha256);
  }
  return policy;
}

/** A tool on PATH, as an absolute path, without running it. */
export function which(tool: string, env: Env = process.env): string | undefined {
  if (tool.includes("/")) return existsSync(tool) ? tool : undefined;
  for (const directory of (env.PATH ?? "").split(delimiter)) {
    if (!directory) continue;
    const candidate = join(directory, tool);
    try { if (statSync(candidate).isFile()) return candidate; } catch { /* not here */ }
  }
  return undefined;
}

export function requiredTools(profile: MobileReleaseProfile): string[] {
  return profile === "github" ? ["unzip", "aapt2", "apksigner", "java"] : profile === "production" ? ["unzip", "bundletool", "jarsigner", "keytool", "java"] : ["unzip", "plutil", "codesign"];
}

/** Missing public authority or inspection tools fail before credentials, cloud work or candidate bytes. */
export function preflightMobileTrust(profile: MobileReleaseProfile, value: unknown, env: Env = process.env): PublicTrustPolicy {
  const policy = validatePublicTrustPolicy(value, profile);
  if (profile === "testflight") throw new Error(IPA_UNSUPPORTED);
  for (const tool of requiredTools(profile)) if (!which(tool, env)) throw new Error(`Binary inspection tool unavailable: ${tool}`);
  return policy;
}

/** Server releases need no mobile policy; all-platform releases need all three and then stop at the IPA gate. */
export function preflightReleaseTrust(scope: "server" | "all", policies: PublicTrustPolicies): void {
  if (scope === "server") return;
  for (const profile of MOBILE_PROFILES) validatePublicTrustPolicy(policies[profile], profile);
  preflightMobileTrust("testflight", policies.testflight);
}

/** Splits `--public-trust-policy <file>` (only as the final pair) from the arguments and reads the policy map. */
export function publicTrustArguments(args: string[]): { args: string[]; policies: PublicTrustPolicies } {
  const index = args.indexOf("--public-trust-policy");
  if (index < 0) return { args, policies: {} };
  if (index !== args.length - 2 || !args[index + 1]) throw new Error("--public-trust-policy <public-json-path> must be the final argument");
  const path = args[index + 1]!;
  if (statSync(path).size > MAX_POLICY_BYTES) throw new Error("Public trust policy exceeds 64 KiB");
  const value = JSON.parse(readFileSync(path, "utf8"));
  if (!value || typeof value !== "object" || Array.isArray(value) || Object.keys(value).some(key => !(MOBILE_PROFILES as readonly string[]).includes(key))) throw new Error("Invalid public trust profile map");
  const policies: PublicTrustPolicies = {};
  for (const profile of MOBILE_PROFILES) {
    if (Object.hasOwn(value, profile)) Object.assign(policies, { [profile]: validatePublicTrustPolicy(value[profile], profile) });
  }
  return { args: args.slice(0, index), policies };
}

/** `apksigner verify --verbose --print-certs`: exactly one signer count and that many ordered records. */
export function apkCurrentSigners(output: string): string[] {
  const lines = output.split(/\r?\n/);
  const counts = lines.filter(line => line.startsWith("Number of signers:"));
  const count = counts.length === 1 ? Number(/^Number of signers: ([1-9]\d*)$/.exec(counts[0]!)?.[1] ?? 0) : 0;
  const records = lines.filter(line => /^Signer #.* certificate SHA-256 digest:/.test(line));
  if (!Number.isSafeInteger(count) || !count || records.length !== count) throw new Error("Incomplete APK signer enumeration");
  return records.map((line, index) => {
    const match = line.match(/^Signer #([1-9]\d*) certificate SHA-256 digest: ([a-fA-F0-9]{64})$/);
    if (!match || Number(match[1]) !== index + 1) throw new Error("Incomplete APK signer enumeration");
    return match[2]!.toLowerCase();
  });
}

/** SHA-256 fingerprints of the PEM certificates `keytool -printcert -rfc -jarfile` prints. */
export function pemCertificateDigests(text: string): string[] {
  const pems = text.match(/-----BEGIN CERTIFICATE-----[\s\S]*?-----END CERTIFICATE-----/g) ?? [];
  return pems.map(pem => new X509Certificate(pem).fingerprint256.replaceAll(":", "").toLowerCase());
}

// Argument vectors only, finite time and output, a fixed locale. A missing tool or nonzero exit refuses.
function inspect(args: string[], env: Env, maxBuffer = 1024 * 1024, input?: Uint8Array): string {
  const tool = which(args[0]!, env);
  if (!tool) throw new Error(`Binary inspection tool unavailable: ${args[0]}`);
  const result = spawnSync(tool, args.slice(1), { env: { PATH: env.PATH ?? "", ...(env.JAVA_HOME ? { JAVA_HOME: env.JAVA_HOME } : {}), LC_ALL: "C", LANG: "C" }, input, timeout: 30_000, maxBuffer });
  if (result.error || result.status !== 0) throw new Error(`Binary inspection tool failed: ${args[0]}`);
  return result.stdout.toString();
}

/** The signer gate for one admitted snapshot. Identity is read separately (`readNativeSnapshot`). */
export function verifyNativeTrust(snapshot: Pick<ArtifactSnapshot, "path">, profile: MobileReleaseProfile, value: unknown, env: Env = process.env): void {
  const policy = validatePublicTrustPolicy(value, profile);
  const path = snapshot.path;
  if (policy.profile === "github") {
    approveSigners(apkCurrentSigners(inspect(["apksigner", "verify", "--verbose", "--print-certs", path], env)), policy.signerSha256);
  } else if (policy.profile === "production") {
    const signatures = inspect(["jarsigner", "-J-Duser.language=en", "-J-Duser.country=US", "-verify", "-verbose", "-certs", path], env);
    // jarsigner can exit zero for unsigned or partly unsigned archives; it is not the signer gate.
    if (!signatures.includes("jar verified.") || signatures.includes("unsigned entries")) throw new Error("AAB signature verification failed");
    // The upload signer, not the Play app-signing identity. Several signers or chains are refused, not guessed.
    approveSigners(pemCertificateDigests(inspect(["keytool", "-printcert", "-rfc", "-jarfile", path], env)), [policy.uploadSignerSha256]);
  } else {
    // Signature integrity and signed claims are necessary but are not Apple distribution authorization.
    if (process.platform === "darwin") {
      const names = inspect(["unzip", "-Z1", path], env).trimEnd().split("\n");
      const apps = names.filter(name => /^Payload\/[^/]+\.app\/Info\.plist$/.test(name));
      const extensions = names.filter(name => /^Payload\/[^/]+\.app\/PlugIns\/[^/]+\.appex\/Info\.plist$/.test(name));
      if (apps.length !== 1 || extensions.length !== 1) throw new Error("IPA main app or notification module missing or ambiguous");
      const temporary = mkdtempSync(join(tmpdir(), "beans-ipa-integrity-"));
      try {
        inspect(["unzip", "-qq", path, "-d", temporary], env);
        for (const [member, identifier] of [[extensions[0]!, "ai.amoena.beans.notify"], [apps[0]!, "ai.amoena.beans"]] as const) {
          const target = join(temporary, member.slice(0, -"/Info.plist".length));
          inspect(["/usr/bin/codesign", "--verify", "--strict", "--all-architectures", "--deep", target], env);
          const entitlements = inspect(["/usr/bin/codesign", "--display", "--entitlements", "-", "--xml", target], env, 64 * 1024);
          inspectIpaSignedClaims(inspect(["/usr/bin/plutil", "-convert", "json", "-o", "-", "-"], env, 64 * 1024, Buffer.from(entitlements)), identifier, policy.teamId);
        }
      } finally { rmSync(temporary, { recursive: true, force: true }); }
    }
    throw new Error(IPA_UNSUPPORTED);
  }
}

/** A candidate mobile binary, inspected from one admitted snapshot: identity, then the signer gate. */
export type MobileBinaryVerifier = (bytes: Uint8Array, profile: MobileReleaseProfile, version: string, buildNumber: string) => Promise<void>;

export function publicTrustVerifier(policies: PublicTrustPolicies, env: Env = process.env): MobileBinaryVerifier {
  return async (bytes, profile, version, buildNumber) => {
    const policy = validatePublicTrustPolicy(policies[profile], profile);
    if (profile === "testflight") throw new Error(IPA_UNSUPPORTED);
    const snapshot = snapshotBytes(bytes);
    try {
      const native = readNativeSnapshot(snapshot, profile, env);
      if (native.identifier !== "ai.amoena.beans" || native.version !== version || native.buildNumber !== buildNumber) {
        throw new Error(`${profile === "github" ? "APK" : "AAB"} native identity differs from release`);
      }
      verifyNativeTrust(snapshot, profile, policy, env);
    } finally { snapshot.dispose(); }
  };
}
