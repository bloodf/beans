import { expect, spyOn, test } from "bun:test";
import { createHash, X509Certificate } from "node:crypto";
import { mkdir, mkdtemp, readFile, rm, writeFile, copyFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { readVersion } from "./app.ts";
import { runEasRelease, stageEasBuild, verifyEasBinary } from "./release-eas.ts";
import { buildReleaseManifest, writeReleaseManifest } from "./release-github.ts";
import { apkCurrentSigners, preflightMobileTrust, type PublicTrustPolicy } from "./release-build-inputs.ts";

const sha = (bytes: Uint8Array | string) => createHash("sha256").update(bytes).digest("hex");
function run(args: string[]) {
  const p = Bun.spawnSync(args, { stdout: "pipe", stderr: "pipe", timeout: 30_000 });
  if (p.exitCode !== 0) throw new Error(`Fixture command failed: ${args[0]}: ${p.stderr}`);
}

test("test-only signed APK accepts explicit pin and rejects signer, identity, hash and absent policy at real boundaries", async () => {
  const root = await mkdtemp(join(tmpdir(), "beans-public-trust-"));
  const version = readVersion(), revision = "a".repeat(40), project = "11111111-1111-4111-8111-111111111111";
  const previous = process.env.BEANS_EXPO_PROJECT_ID;
  process.env.BEANS_EXPO_PROJECT_ID = project;
  let fetcher: ReturnType<typeof spyOn> | undefined;
  try {
    // Ephemeral fixture key only. Never operator trust or persistent signing credentials.
    const key = join(root, "test-only.p12"), cert = join(root, "test-only.pem");
    run(["keytool", "-genkeypair", "-keystore", key, "-storepass", "fixture-only", "-keypass", "fixture-only", "-alias", "fixture", "-keyalg", "RSA", "-keysize", "2048", "-validity", "2", "-dname", "CN=Beans TEST ONLY"]);
    run(["keytool", "-exportcert", "-rfc", "-keystore", key, "-storepass", "fixture-only", "-alias", "fixture", "-file", cert]);
    const policy: PublicTrustPolicy = { profile: "github", rotation: "none", signerSha256: [new X509Certificate(await readFile(cert)).fingerprint256.replaceAll(":", "").toLowerCase()] };
    async function apk(identifier: string, label: string) {
      const manifest = join(root, `${label}.xml`), unsigned = join(root, `${label}-unsigned.apk`), signed = join(root, `${label}.apk`);
      await writeFile(manifest, `<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="${identifier}" android:versionCode="42" android:versionName="${version}"><uses-sdk android:minSdkVersion="24" android:targetSdkVersion="34"/><application android:hasCode="false"/></manifest>`);
      run(["aapt2", "link", "-I", "/Users/heitor/Library/Android/sdk/platforms/android-34/android.jar", "--manifest", manifest, "-o", unsigned]);
      run(["apksigner", "sign", "--ks", key, "--ks-pass", "pass:fixture-only", "--key-pass", "pass:fixture-only", "--out", signed, unsigned]);
      return new Uint8Array(await readFile(signed));
    }
    const bytes = await apk("ai.amoena.beans", "valid"), wrongIdentity = await apk("ai.amoena.beans.dev", "wrong-identity");
    await verifyEasBinary(bytes, "github", version, "42", policy);
    await expect(verifyEasBinary(bytes, "github", version, "42")).rejects.toThrow("policy unavailable");
    const wrongSigner: PublicTrustPolicy = { ...policy, signerSha256: ["0".repeat(64)] };
    await expect(verifyEasBinary(bytes, "github", version, "42", wrongSigner)).rejects.toThrow("signer differs");
    await expect(verifyEasBinary(wrongIdentity, "github", version, "42", policy)).rejects.toThrow("native identity");
    await expect(verifyEasBinary(bytes, "github", version, "43", policy)).rejects.toThrow("native identity");
    const corrupted = bytes.slice(); corrupted[50] ^= 1;
    await expect(verifyEasBinary(corrupted, "github", version, "42", policy)).rejects.toThrow();
    const build = { id: "22222222-2222-4222-8222-222222222222", status: "FINISHED", platform: "ANDROID", appIdentifier: "ai.amoena.beans", distribution: "STORE", buildProfile: "github", gitCommitHash: revision, appVersion: version, appBuildVersion: "42", app: { id: project }, artifacts: { buildUrl: "https://artifacts.eascdn.net/test-only.apk" } };
    fetcher = spyOn(globalThis, "fetch").mockImplementation(async () => new Response(bytes) as any);
    const inventory = join(root, "inventory");
    await stageEasBuild(build, "github", revision, inventory, policy);
    const proof = JSON.parse(await readFile(join(inventory, "eas-github.json"), "utf8"));
    expect(proof.sha256).toBe(sha(bytes));
    const policyFile = join(root, "public-policy.json");
    await writeFile(policyFile, JSON.stringify({ github: policy }));
    let queried = false;
    const query = async () => { queried = true; return [build]; };
    const git = (args: string[]) => ({ exitCode: 0, stdout: Buffer.from(args.includes("rev-parse") ? revision : "") }) as any;
    await expect(runEasRelease(["build", "github", revision, join(root, "no-policy")], query, git)).rejects.toThrow("policy unavailable");
    expect(queried).toBe(false);
    const oldToken = process.env.EXPO_TOKEN, oldOwner = process.env.BEANS_EXPO_OWNER;
    process.env.EXPO_TOKEN = "fixture-only"; process.env.BEANS_EXPO_OWNER = "fixture-only";
    try {
      await runEasRelease(["build", "github", revision, join(root, "caller-positive"), "--public-trust-policy", policyFile], query, git);
      expect(JSON.parse(await readFile(join(root, "caller-positive", "eas-github.json"), "utf8")).sha256).toBe(sha(bytes));
    } finally {
      if (oldToken === undefined) delete process.env.EXPO_TOKEN; else process.env.EXPO_TOKEN = oldToken;
      if (oldOwner === undefined) delete process.env.BEANS_EXPO_OWNER; else process.env.BEANS_EXPO_OWNER = oldOwner;
    }
    const allPolicies = { github: policy, production: { profile: "production", uploadSignerSha256: policy.signerSha256[0], rotation: "none" }, testflight: { profile: "testflight", teamId: "TESTONLY00", certificateSha256: policy.signerSha256 } } as const;
    const allFile = join(root, "all-public-policy.json");
    await writeFile(allFile, JSON.stringify(allPolicies));
    queried = false;
    await expect(runEasRelease(["build", "testflight", revision, join(root, "ipa"), "--public-trust-policy", allFile], query, git)).rejects.toThrow("IPA cryptographic trust verification unsupported");
    expect(queried).toBe(false);
    await expect(writeReleaseManifest(`beans-v${version}`, revision, inventory, "all", allPolicies as any)).rejects.toThrow("IPA cryptographic trust verification unsupported");
    for (const command of ["check", "manifest", "publish"]) {
      const positional = command === "check" ? [`beans-v${version}`, "all"] : [`beans-v${version}`, revision, inventory, "all"];
      const child = Bun.spawnSync([process.execPath, "scripts/release-github.ts", command, ...positional, "--public-trust-policy", allFile], { stdout: "pipe", stderr: "pipe", timeout: 10_000 });
      expect(child.exitCode).not.toBe(0);
      expect(child.stderr.toString()).toContain("IPA cryptographic trust verification unsupported");
    }
    const enumeration = `Number of signers: 1\nSigner #1 certificate SHA-256 digest: ${policy.signerSha256[0]}\n`;
    expect(apkCurrentSigners(enumeration)).toEqual(policy.signerSha256);
    for (const malformed of [enumeration.replace("signers: 1", "signers: 2"), enumeration.replace("Signer #1", "Signer #2"), enumeration + enumeration, enumeration.replace(policy.signerSha256[0], "truncated")]) expect(() => apkCurrentSigners(malformed)).toThrow("Incomplete APK signer enumeration");
    if (!Bun.which("bundletool")) expect(() => preflightMobileTrust("production", allPolicies.production)).toThrow("tool unavailable: bundletool");
    await expect(stageEasBuild(build, "github", revision, join(root, "rejected"), wrongSigner)).rejects.toThrow("signer differs");
    expect(await Bun.file(join(root, "rejected", "eas-github.json")).exists()).toBe(false);
    const names = ["beans-server-linux-x86_64.tar.gz", "beans-server-linux-aarch64.tar.gz", "beans-server-updater.py", `Beans-${version}.zip`, `Beans-${version}.dmg`, "appcast.xml", `Beans Setup ${version}.exe`, `beans_${version}_amd64.deb`, `beans_${version}_arm64.deb`, "install-linux-amd64.sh", "install-linux-arm64.sh", ...["windows-amd64", "linux-amd64", "linux-arm64"].flatMap(p => [`update-${p}.json`, `beans-${version}-${p}.tar.gz`])];
    for (const name of names) await writeFile(join(inventory, name), "fixture");
    for (const suffix of ["macos-aarch64.tar.gz", "linux-aarch64.tar.gz", "linux-x86_64.tar.gz", "windows-x86_64.zip"]) {
      const name = `beans-cli-${suffix}`;
      await writeFile(join(inventory, name), "fixture");
      await writeFile(join(inventory, `${name}.sha256`), `${sha("fixture")}  ${name}\n`);
    }
    for (const [index, profile] of ["production", "testflight"].entries()) {
      const artifact = `Beans-${version}${profile === "production" ? ".aab" : "-store.ipa"}`;
      await writeFile(join(inventory, artifact), "fixture");
      await writeFile(join(inventory, `eas-${profile}.json`), JSON.stringify({ ...proof, profile, artifact, size: 7, sha256: sha("fixture"), buildId: `${index + 3}2222222-2222-4222-8222-222222222222`, platform: profile === "production" ? "ANDROID" : "IOS" }));
    }
    await expect(buildReleaseManifest(`beans-v${version}`, revision, inventory, "all", { github: wrongSigner })).rejects.toThrow("signer differs");
    // Valid APK passes finalization inspection; next invalid AAB blocks, never fake all success.
    await expect(buildReleaseManifest(`beans-v${version}`, revision, inventory, "all", { github: policy })).rejects.toThrow("archive structure");
    await writeFile(join(inventory, `Beans-${version}.apk`), wrongIdentity);
    await expect(buildReleaseManifest(`beans-v${version}`, revision, inventory, "all", { github: policy })).rejects.toThrow("provenance: github");
    if (process.env.BEANS_TEST_TRUST_ARTIFACTS) {
      const output = process.env.BEANS_TEST_TRUST_ARTIFACTS;
      await mkdir(output, { recursive: true });
      await writeFile(join(output, "valid-test-only.apk"), bytes);
      await writeFile(join(output, "wrong-identity-test-only.apk"), wrongIdentity);
      await copyFile(cert, join(output, "test-only-public.pem"));
      await writeFile(join(output, "test-only-policy.json"), JSON.stringify(policy));
      console.log(JSON.stringify({ fixtureOnly: true, validSha256: sha(bytes), wrongIdentitySha256: sha(wrongIdentity), signerSha256: policy.signerSha256[0], output }));
    }
  } finally {
    fetcher?.mockRestore();
    if (previous === undefined) delete process.env.BEANS_EXPO_PROJECT_ID; else process.env.BEANS_EXPO_PROJECT_ID = previous;
    await rm(root, { recursive: true, force: true });
  }
});
