import { spawnSync } from "node:child_process";
import { X509Certificate } from "node:crypto";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { afterEach, describe, expect, test, vi } from "vitest";
import { readVersion, ROOT } from "./app.ts";
import { androidBuildTools, androidSdk } from "./android-fixture.ts";
import { runEasRelease, runLocalEasRelease, stageEasBuild } from "./release-eas.ts";
import { inspectIpaSignedClaims } from "./release-ipa-claims.ts";
import { snapshotBytes } from "./release-native-identity.ts";
import {
  apkCurrentSigners, approveSigners, IPA_UNSUPPORTED, pemCertificateDigests, preflightMobileTrust, preflightReleaseTrust,
  publicTrustArguments, publicTrustVerifier, validatePublicTrustPolicy, verifyNativeTrust, which, type PublicTrustPolicies,
} from "./release-trust.ts";

const pin = (c: string) => c.repeat(64);
const github = { profile: "github", signerSha256: [pin("a")], rotation: "none" } as const;
const production = { profile: "production", uploadSignerSha256: pin("b"), rotation: "none" } as const;
const testflight = { profile: "testflight", teamId: "TESTONLY00", certificateSha256: [pin("c")] } as const;
const all: PublicTrustPolicies = { github, production, testflight };
const dirs: string[] = [];
afterEach(async () => { await Promise.all(dirs.splice(0).map((dir) => rm(dir, { recursive: true, force: true }))); });
async function scratch(prefix: string) { const dir = await mkdtemp(join(tmpdir(), prefix)); dirs.push(dir); return dir; }

describe("public trust policy", () => {
  test("accepts each profile's exact shape and refuses every other", () => {
    expect(validatePublicTrustPolicy(github, "github")).toBe(github);
    expect(validatePublicTrustPolicy(production, "production")).toBe(production);
    expect(validatePublicTrustPolicy(testflight, "testflight")).toBe(testflight);
    for (const [value, profile] of [
      [undefined, "github"], [null, "github"], [[], "github"], ["x", "github"], [production, "github"], [github, "production"],
      [{ ...github, rotation: "allowed" }, "github"], [{ ...github, signerSha256: [] }, "github"], [{ ...github, signerSha256: [pin("A")] }, "github"],
      [{ ...github, signerSha256: [pin("a"), pin("a")] }, "github"], [{ ...github, signerSha256: pin("a") }, "github"], [{ ...github, extra: true }, "github"],
      [{ ...production, uploadSignerSha256: "short" }, "production"], [{ ...production, uploadSignerSha256: [pin("b")] }, "production"],
      [{ ...testflight, teamId: "lowercase0" }, "testflight"], [{ ...testflight, certificateSha256: [] }, "testflight"],
    ] as const) expect(() => validatePublicTrustPolicy(value, profile), JSON.stringify(value)).toThrow();
  });

  test("a candidate signer must equal the approved set exactly", () => {
    approveSigners([pin("a"), pin("b")], [pin("b"), pin("a")]);
    for (const actual of [[], [pin("a")], [pin("a"), pin("b"), pin("c")], [pin("a"), pin("a")], [pin("c"), pin("d")]]) {
      expect(() => approveSigners(actual, [pin("a"), pin("b")])).toThrow("signer differs");
    }
  });

  test("--public-trust-policy is only the final pair, bounded, and names only mobile profiles", async () => {
    const dir = await scratch("beans-trust-args-");
    const file = join(dir, "policy.json");
    await writeFile(file, JSON.stringify({ github, production }));
    expect(publicTrustArguments(["build", "github"])).toEqual({ args: ["build", "github"], policies: {} });
    expect(publicTrustArguments(["build", "github", "--public-trust-policy", file])).toEqual({ args: ["build", "github"], policies: { github, production } });
    expect(() => publicTrustArguments(["--public-trust-policy", file, "build"])).toThrow("final argument");
    expect(() => publicTrustArguments(["build", "--public-trust-policy"])).toThrow("final argument");
    for (const body of [JSON.stringify({ desktop: github }), JSON.stringify([github]), "null", JSON.stringify({ github: production })]) {
      await writeFile(file, body);
      expect(() => publicTrustArguments(["x", "--public-trust-policy", file]), body).toThrow();
    }
    await writeFile(file, " ".repeat(64 * 1024 + 1));
    expect(() => publicTrustArguments(["x", "--public-trust-policy", file])).toThrow("64 KiB");
  });
});

describe("preflight", () => {
  test("refuses a missing policy before tools, and missing tools before any work", () => {
    expect(() => preflightMobileTrust("github", undefined, { PATH: "" })).toThrow("policy unavailable for github");
    expect(() => preflightMobileTrust("github", github, { PATH: "" })).toThrow("tool unavailable: unzip");
    expect(() => preflightMobileTrust("production", production, { PATH: "" })).toThrow("tool unavailable");
  });

  test("IPA publication stays blocked even with an explicit policy", () => {
    expect(() => preflightMobileTrust("testflight", testflight)).toThrow(IPA_UNSUPPORTED);
    expect(() => preflightMobileTrust("testflight", undefined)).toThrow("policy unavailable for testflight");
  });

  test("server releases need no mobile policy; all-platform needs all three and still stops at IPA", () => {
    preflightReleaseTrust("server", {});
    expect(() => preflightReleaseTrust("all", {})).toThrow("policy unavailable for github");
    expect(() => preflightReleaseTrust("all", { github, production })).toThrow("policy unavailable for testflight");
    expect(() => preflightReleaseTrust("all", all)).toThrow(IPA_UNSUPPORTED);
  });

  test("which finds tools on the given PATH only", async () => {
    const dir = await scratch("beans-which-");
    await writeFile(join(dir, "tool"), "");
    expect(which("tool", { PATH: dir })).toBe(join(dir, "tool"));
    expect(which("tool", { PATH: "" })).toBeUndefined();
  });
});

test("apksigner enumeration needs one count and exactly that many ordered records", () => {
  const record = (n: number, digest: string) => `Signer #${n} certificate SHA-256 digest: ${digest}`;
  const one = `Verifies\nNumber of signers: 1\n${record(1, pin("A"))}\n`;
  expect(apkCurrentSigners(one)).toEqual([pin("a")]);
  expect(apkCurrentSigners(`Number of signers: 2\n${record(1, pin("a"))}\n${record(2, pin("b"))}\n`)).toEqual([pin("a"), pin("b")]);
  for (const malformed of [one.replace("signers: 1", "signers: 2"), one.replace("Signer #1", "Signer #2"), one + one, one.replace(pin("A"), "truncated"), "Number of signers: 0\n", ""]) {
    expect(() => apkCurrentSigners(malformed), malformed).toThrow("Incomplete APK signer enumeration");
  }
});

describe("callers fail closed before credentials, cloud work or candidate bytes", () => {
  const revision = "a".repeat(40);
  const git = (args: string[]) => ({ exitCode: 0, stdout: Buffer.from(args.includes("rev-parse") ? revision : "") }) as any;

  test("runEasRelease and runLocalEasRelease refuse without a policy and never query or build", async () => {
    const dir = await scratch("beans-trust-callers-");
    const query = vi.fn(async () => []);
    const builder = vi.fn(() => ({ exitCode: 0 }));
    for (const profile of ["github", "production", "testflight"]) {
      await expect(runEasRelease(["build", profile, revision, dir], query, git)).rejects.toThrow(`policy unavailable for ${profile}`);
      await expect(runLocalEasRelease([profile, revision, dir], undefined, builder, git)).rejects.toThrow(`policy unavailable for ${profile}`);
    }
    const file = join(dir, "policy.json");
    await writeFile(file, JSON.stringify(all));
    await expect(runEasRelease(["build", "testflight", revision, dir, "--public-trust-policy", file], query, git)).rejects.toThrow(IPA_UNSUPPORTED);
    expect(query).not.toHaveBeenCalled();
    expect(builder).not.toHaveBeenCalled();
  });

  test("stageEasBuild without a verifier downloads nothing; a refusing verifier stages nothing", async () => {
    const dir = await scratch("beans-trust-stage-");
    const previous = process.env.BEANS_EXPO_PROJECT_ID;
    const project = "11111111-1111-4111-8111-111111111111";
    process.env.BEANS_EXPO_PROJECT_ID = project;
    const fetcher = vi.spyOn(globalThis, "fetch").mockImplementation(async () => new Response(new Uint8Array([0x50, 0x4b, 3, 4, 1])) as any);
    try {
      const build = { id: "22222222-2222-4222-8222-222222222222", status: "FINISHED", platform: "ANDROID", appIdentifier: "ai.amoena.beans", distribution: "STORE", buildProfile: "github", gitCommitHash: revision, appVersion: readVersion(), appBuildVersion: "42", app: { id: project }, artifacts: { buildUrl: "https://artifacts.eascdn.net/build.apk" } };
      await expect(stageEasBuild(build, "github", revision, dir, undefined as any)).rejects.toThrow("policy unavailable for github");
      expect(fetcher).not.toHaveBeenCalled();
      await expect(stageEasBuild(build, "github", revision, dir, async () => { throw new Error("Binary signer differs from approved public trust policy"); })).rejects.toThrow("signer differs");
      expect(existsSync(join(dir, "eas-github.json"))).toBe(false);
      expect(existsSync(join(dir, `Beans-${readVersion()}.apk`))).toBe(false);
    } finally {
      fetcher.mockRestore();
      if (previous === undefined) delete process.env.BEANS_EXPO_PROJECT_ID; else process.env.BEANS_EXPO_PROJECT_ID = previous;
    }
  });

  test("the verifier refuses IPA and a policy for another profile before reading bytes", async () => {
    const verify = publicTrustVerifier({ github, testflight });
    await expect(verify(new Uint8Array([1]), "testflight", readVersion(), "1")).rejects.toThrow(IPA_UNSUPPORTED);
    await expect(verify(new Uint8Array([1]), "production", readVersion(), "1")).rejects.toThrow("policy unavailable for production");
    await expect(verify(new Uint8Array([0x50, 0x4b]), "github", readVersion(), "1")).rejects.toThrow("Unsafe binary archive");
  });

  test("release-github check/manifest/publish refuse before notes, keys or network", () => {
    const loader = (() => { try { return import.meta.resolve("tsx"); } catch { return pathToFileURL(resolve(ROOT, "node_modules/tsx/dist/loader.mjs")).href; } })();
    const env = { ...process.env, BEANS_UPDATE_PRIVATE_KEY: "", GH_TOKEN: "", GITHUB_TOKEN: "" };
    delete env.NODE_OPTIONS;
    const tag = `beans-v${readVersion()}`;
    for (const command of ["check", "manifest", "publish"]) {
      const positional = command === "check" ? [tag, "all"] : [tag, "a".repeat(40), tmpdir(), "all"];
      const child = spawnSync(process.execPath, ["--import", loader, join(ROOT, "scripts", "release-github.ts"), command, ...positional], { cwd: ROOT, env, encoding: "utf8", timeout: 30_000 });
      expect(child.status, child.stderr).not.toBe(0);
      expect(child.stderr).toContain("trust policy unavailable for github");
    }
  });
});

// A real signed archive through the JDK's own jarsigner and keytool: the upload-signer gate of an
// AAB does not depend on bundletool, so it runs wherever a JDK is installed.
describe.skipIf(!which("jarsigner") || !which("keytool") || !which("zip"))("AAB upload signer with a test-only key", () => {
  function run(args: string[], cwd?: string) {
    const child = spawnSync(args[0]!, args.slice(1), { cwd, encoding: "utf8", timeout: 60_000 });
    if (child.status !== 0) throw new Error(`fixture command failed: ${args[0]}: ${child.stderr}`);
    return child.stdout;
  }
  test("accepts the pinned upload signer and refuses another pin, an unsigned archive and unsigned additions", async () => {
    const dir = await scratch("beans-aab-trust-");
    const key = join(dir, "upload-test-only.p12"), cert = join(dir, "upload.pem");
    run(["keytool", "-genkeypair", "-keystore", key, "-storetype", "PKCS12", "-storepass", "fixture-only", "-keypass", "fixture-only", "-alias", "upload", "-keyalg", "RSA", "-keysize", "2048", "-validity", "2", "-dname", "CN=Beans AAB UPLOAD TEST ONLY"]);
    run(["keytool", "-exportcert", "-rfc", "-keystore", key, "-storepass", "fixture-only", "-alias", "upload", "-file", cert]);
    const upload = new X509Certificate(await readFile(cert)).fingerprint256.replaceAll(":", "").toLowerCase();
    expect(pemCertificateDigests(await readFile(cert, "utf8"))).toEqual([upload]);
    const module = join(dir, "module");
    await mkdir(join(module, "base", "manifest"), { recursive: true });
    await writeFile(join(module, "base", "manifest", "AndroidManifest.xml"), "test-only manifest bytes");
    const unsigned = join(dir, "unsigned.aab"), signed = join(dir, "signed.aab"), added = join(dir, "added.aab");
    run(["zip", "-q", "-r", unsigned, "base"], module);
    run(["cp", unsigned, signed]);
    run(["jarsigner", "-keystore", key, "-storetype", "PKCS12", "-storepass", "fixture-only", "-keypass", "fixture-only", signed, "upload"]);
    const policy = { ...production, uploadSignerSha256: upload };
    verifyNativeTrust({ path: signed }, "production", policy);
    expect(() => verifyNativeTrust({ path: signed }, "production", { ...policy, uploadSignerSha256: pin("0") })).toThrow("signer differs");
    expect(() => verifyNativeTrust({ path: unsigned }, "production", policy)).toThrow();
    run(["cp", signed, added]);
    await mkdir(join(dir, "base", "assets"), { recursive: true });
    await writeFile(join(dir, "base", "assets", "unsigned.txt"), "Unsigned content added after upload signing.\n");
    run(["zip", "-q", added, "base/assets/unsigned.txt"], dir);
    expect(() => verifyNativeTrust({ path: added }, "production", policy)).toThrow("AAB signature verification failed");
  }, 120_000);
});

// A real signed APK needs Android build-tools (aapt2, apksigner); CI installs them (#596).
const tools = androidBuildTools();
describe.skipIf(tools.skip)("APK current signer with a test-only key", () => {
  test("accepts the pinned signer and refuses another signer, identity or build number", async () => {
    if (!tools.available) throw new Error(tools.reason);
    const env = { ...process.env, PATH: `${tools.path}:${process.env.PATH}` };
    const run = (args: string[]) => { const child = spawnSync(args[0]!, args.slice(1), { env, encoding: "utf8", timeout: 60_000 }); if (child.status !== 0) throw new Error(`fixture command failed: ${args[0]}: ${child.stderr}`); };
    const dir = await scratch("beans-apk-trust-");
    const key = join(dir, "test-only.p12"), cert = join(dir, "test-only.pem"), version = readVersion();
    run(["keytool", "-genkeypair", "-keystore", key, "-storetype", "PKCS12", "-storepass", "fixture-only", "-keypass", "fixture-only", "-alias", "fixture", "-keyalg", "RSA", "-keysize", "2048", "-validity", "2", "-dname", "CN=Beans TEST ONLY"]);
    run(["keytool", "-exportcert", "-rfc", "-keystore", key, "-storepass", "fixture-only", "-alias", "fixture", "-file", cert]);
    const signer = new X509Certificate(await readFile(cert)).fingerprint256.replaceAll(":", "").toLowerCase();
    async function apk(identifier: string, label: string) {
      const manifest = join(dir, `${label}.xml`), unsigned = join(dir, `${label}-unsigned.apk`), signed = join(dir, `${label}.apk`);
      await writeFile(manifest, `<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="${identifier}" android:versionCode="42" android:versionName="${version}"><uses-sdk android:minSdkVersion="24" android:targetSdkVersion="34"/><application android:hasCode="false"/></manifest>`);
      run([join(tools.path, "aapt2"), "link", "-I", join(androidSdk(), "platforms/android-34/android.jar"), "--manifest", manifest, "-o", unsigned]);
      run([join(tools.path, "apksigner"), "sign", "--ks", key, "--ks-pass", "pass:fixture-only", "--key-pass", "pass:fixture-only", "--out", signed, unsigned]);
      return new Uint8Array(await readFile(signed));
    }
    const bytes = await apk("ai.amoena.beans", "valid"), other = await apk("ai.amoena.beans.dev", "other");
    const policy = { ...github, signerSha256: [signer] };
    const verify = publicTrustVerifier({ github: policy }, env);
    await verify(bytes, "github", version, "42");
    await expect(publicTrustVerifier({ github: { ...policy, signerSha256: [pin("0")] } }, env)(bytes, "github", version, "42")).rejects.toThrow("signer differs");
    await expect(verify(other, "github", version, "42")).rejects.toThrow("native identity");
    await expect(verify(bytes, "github", version, "43")).rejects.toThrow("native identity");
    const snapshot = snapshotBytes(bytes);
    try { verifyNativeTrust(snapshot, "github", policy, env); } finally { snapshot.dispose(); }
  }, 120_000);
});

describe("IPA signed claims", () => {
  const teamId = "TEAMONLY00", prefix = "PREFIX0000";
  const claims = (bundle = "ai.amoena.beans") => ({
    "application-identifier": `${prefix}.${bundle}`, "com.apple.developer.team-identifier": teamId,
    "keychain-access-groups": [`${prefix}.${bundle}`], "com.apple.security.application-groups": ["group.ai.amoena.beans"], "get-task-allow": false,
  });
  const inspect = (value: unknown, bundle = "ai.amoena.beans") => inspectIpaSignedClaims(JSON.stringify(value), bundle, teamId);

  test("bind the exact bundle without equating the App ID prefix and the Team ID", () => {
    for (const bundle of ["ai.amoena.beans", "ai.amoena.beans.notify"]) {
      const result = inspect(claims(bundle), bundle);
      expect(result).toEqual({ teamId, applicationIdentifier: `${prefix}.${bundle}`, applicationIdentifierPrefix: prefix, bundleIdentifier: bundle });
      expect(Object.isFrozen(result)).toBe(true);
    }
    expect(() => inspect(claims("ai.amoena.beans.notify"))).toThrow("signed entitlement claims");
  });

  test("absent, malformed, oversized and debuggable claims refuse without echoing them", () => {
    const secret = "PRIVATE-CLAIM-CANARY";
    for (const value of [null, [], true, "claims", {},
      { ...claims(), "com.apple.developer.team-identifier": "OTHERTEAM0" }, { ...claims(), "application-identifier": null },
      { ...claims(), "application-identifier": `${prefix}.other` }, { ...claims(), "application-identifier": `short.ai.amoena.beans` },
      { ...claims(), "keychain-access-groups": ["*"] }, { ...claims(), "keychain-access-groups": ["a", "a"] },
      { ...claims(), "get-task-allow": true }, { ...claims(), deep: Array.from({ length: 10 }).reduce<unknown>((inner) => [inner], "x") }, { ...claims(), [secret]: "\u0000" },
    ]) {
      let message = "";
      try { inspect(value); } catch (error) { message = String(error); }
      expect(message, JSON.stringify(value)).toContain("signed entitlement claims");
      expect(message).not.toContain(secret);
    }
    expect(() => inspectIpaSignedClaims("x".repeat(64 * 1024 + 1), "ai.amoena.beans", teamId)).toThrow();
    expect(() => inspectIpaSignedClaims(JSON.stringify(claims()), "ai.amoena.other", teamId)).toThrow();
  });
});
