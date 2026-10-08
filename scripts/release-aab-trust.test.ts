import { expect, test } from "bun:test";
import { X509Certificate, createHash } from "node:crypto";
import { copyFile, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { readVersion } from "./app.ts";
import { inspectMobileBinary, preflightMobileTrust } from "./release-build-inputs.ts";

function run(args: string[], cwd?: string): Buffer {
  const child = Bun.spawnSync(args, { cwd, stdout: "pipe", stderr: "pipe", timeout: 30_000 });
  if (child.exitCode !== 0) throw new Error(`Test-only fixture command failed: ${args[0]}: ${child.stderr}`);
  return Buffer.from(child.stdout);
}

test("official bundletool AAB accepts distinct upload pin and rejects unsigned added content", async () => {
  const root = await mkdtemp(join(tmpdir(), "beans-aab-public-trust-"));
  try {
    const version = readVersion(), key = join(root, "upload-test-only.p12"), cert = join(root, "upload-test-only.pem");
    run(["keytool", "-genkeypair", "-keystore", key, "-storepass", "fixture-only", "-keypass", "fixture-only", "-alias", "upload-fixture", "-keyalg", "RSA", "-keysize", "2048", "-validity", "2", "-dname", "CN=Beans AAB UPLOAD TEST ONLY"]);
    run(["keytool", "-exportcert", "-rfc", "-keystore", key, "-storepass", "fixture-only", "-alias", "upload-fixture", "-file", cert]);
    const pin = new X509Certificate(await readFile(cert)).fingerprint256.replaceAll(":", "").toLowerCase();
    const policy = { profile: "production", uploadSignerSha256: pin, rotation: "none" } as const;
    preflightMobileTrust("production", policy);
    const manifest = join(root, "AndroidManifest.xml"), proto = join(root, "proto.apk"), module = join(root, "module"), base = join(root, "base.zip"), signed = join(root, "signed-test-only.aab");
    await writeFile(manifest, `<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="ai.amoena.beans" android:versionCode="43" android:versionName="${version}"><uses-sdk android:minSdkVersion="24" android:targetSdkVersion="34"/><application android:hasCode="false"/></manifest>`);
    run(["aapt2", "link", "--proto-format", "-I", "/Users/heitor/Library/Android/sdk/platforms/android-34/android.jar", "--manifest", manifest, "-o", proto]);
    await mkdir(join(module, "manifest"), { recursive: true });
    await writeFile(join(module, "manifest", "AndroidManifest.xml"), run(["unzip", "-p", proto, "AndroidManifest.xml"]));
    await writeFile(join(module, "resources.pb"), run(["unzip", "-p", proto, "resources.pb"]));
    run(["zip", "-q", "-r", base, "manifest", "resources.pb"], module);
    run(["bundletool", "build-bundle", `--modules=${base}`, `--output=${signed}`]);
    run(["jarsigner", "-keystore", key, "-storepass", "fixture-only", "-keypass", "fixture-only", signed, "upload-fixture"]);
    inspectMobileBinary(signed, "production", version, "43", policy);
    expect(() => inspectMobileBinary(signed, "production", version, "43", { ...policy, uploadSignerSha256: "0".repeat(64) })).toThrow("signer differs");
    expect(() => inspectMobileBinary(signed, "production", version, "44", policy)).toThrow("native identity");
    const added = join(root, "unsigned-added-test-only.aab");
    await copyFile(signed, added);
    await mkdir(join(root, "base", "assets"), { recursive: true });
    await writeFile(join(root, "base", "assets", "unsigned.txt"), "Unsigned content added after upload signing.\n");
    run(["zip", "-q", added, "base/assets/unsigned.txt"], root);
    expect(() => inspectMobileBinary(added, "production", version, "43", policy)).toThrow("AAB signature verification failed");
    const output = process.env.BEANS_TEST_AAB_ARTIFACTS;
    if (output) {
      await mkdir(output, { recursive: true });
      await copyFile(signed, join(output, "signed-test-only.aab"));
      await copyFile(added, join(output, "unsigned-added-test-only.aab"));
      await copyFile(cert, join(output, "upload-test-only-public.pem"));
      await writeFile(join(output, "upload-test-only-policy.json"), JSON.stringify({ production: policy }));
      const sha = async (path: string) => createHash("sha256").update(await readFile(path)).digest("hex");
      console.log(JSON.stringify({ fixtureOnly: true, uploadSignerSha256: pin, signedSha256: await sha(signed), unsignedAddedSha256: await sha(added), bundletoolVersion: run(["bundletool", "version"]).toString().trim(), output }));
    }
  } finally { await rm(root, { recursive: true, force: true }); }
}, 30_000);
