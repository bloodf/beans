import { expect, test } from "bun:test";
import { createHash, generateKeyPairSync, sign } from "node:crypto";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { readVersion, ROOT } from "./app.ts";
import { submitEas } from "./submit-eas.ts";

test("submission verifies signed provenance and bytes before testing-only upload", async () => {
  const directory = await mkdtemp(join(tmpdir(), "beans-submit-test-"));
  const v = readVersion(), revision = "a".repeat(40), project = "11111111-1111-4111-8111-111111111111";
  const { publicKey, privateKey } = generateKeyPairSync("ed25519");
  const trust = publicKey.export({ type: "spki", format: "der" }).subarray(-32).toString("base64");
  const binary = new Uint8Array([0x50, 0x4b, 3, 4]);
  const sha = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
  const values = { EXPO_TOKEN: "fixture", BEANS_EXPO_OWNER: "fixture", BEANS_EXPO_PROJECT_ID: project, BEANS_PLAY_SERVICE_ACCOUNT_JSON: JSON.stringify({ type: "service_account" }), BEANS_ASC_APP_ID: "12345", BEANS_ASC_KEY_ID: "ABCDEFGHIJ", BEANS_ASC_ISSUER_ID: project, BEANS_ASC_KEY_P8: "-----BEGIN PRIVATE KEY-----\nfixture" };
  const old = Object.fromEntries(Object.keys(values).map((name) => [name, process.env[name]]));
  Object.assign(process.env, values);
  try {
    for (const platform of ["android", "ios"]) {
      const mobile = join(directory, platform); await mkdir(mobile);
      await writeFile(join(mobile, "eas.json"), await Bun.file(join(ROOT, "mobile/eas.json")).text());
      const profile = platform === "android" ? "production" : "testflight";
      const artifact = `Beans-${v}${platform === "android" ? ".aab" : "-store.ipa"}`;
      const proof = { schema: 1, revision, version: v, project, buildId: "22222222-2222-4222-8222-222222222222", buildNumber: "42", profile, artifact, size: binary.length, sha256: sha(binary) };
      const proofBytes = Buffer.from(JSON.stringify(proof));
      const manifest = Buffer.from(JSON.stringify({ schema: 1, revision, version: v, artifacts: [{ name: `eas-${profile}.json`, size: proofBytes.length, sha256: sha(proofBytes) }, { name: artifact, size: binary.length, sha256: sha(binary) }] }));
      const originalConfig = await Bun.file(join(mobile, "eas.json")).text();
      await mkdir(join(mobile, ".credentials"));
      await writeFile(join(mobile, ".credentials/keep"), "existing");
      let keyPath = "";
      let failSubmit = false;
      let submitted = false;
      const runner = async (args: string[]) => {
        if (args[0] === "git") return revision;
        if (args.includes("download")) {
          const dest = args[args.indexOf("--dir") + 1];
          await writeFile(join(dest, "beans-update.json"), manifest);
          await writeFile(join(dest, "beans-update.json.sig"), sign(null, manifest, privateKey).toString("base64"));
          await writeFile(join(dest, `eas-${profile}.json`), proofBytes);
          return "";
        }
        if (args[0] === "gh") return JSON.stringify(args[2].includes("/commits/") ? { sha: revision } : { tag_name: `beans-v${v}`, draft: false, prerelease: false });
        if (args[1] === "build:view") return JSON.stringify({ id: proof.buildId, status: "FINISHED", platform: platform.toUpperCase(), buildProfile: profile, gitCommitHash: revision, appVersion: v, appBuildVersion: "42", appIdentifier: "ai.amoena.beans", distribution: "STORE", app: { id: project }, artifacts: { buildUrl: "https://artifacts.eascdn.net/store" } });
        if (args[1] === "submit") {
          expect(args).toContain("--path"); expect(args).not.toContain("--latest");
          const config = await Bun.file(join(mobile, "eas.json")).json();
          keyPath = platform === "android" ? config.submit.production.android.serviceAccountKeyPath : config.submit.production.ios.ascApiKeyPath;
          expect(await Bun.file(keyPath).exists()).toBe(true);
          if (failSubmit) throw new Error("fixture submit failed");
          submitted = true; return "";
        }
        throw new Error("Unexpected fixture command");
      };
      await submitEas([`beans-v${v}`, platform], runner, async () => trust, async () => binary, mobile);
      expect(submitted).toBe(true);
      expect(await Bun.file(join(mobile, "eas.json")).text()).toBe(originalConfig);
      expect(await Bun.file(keyPath).exists()).toBe(false);
      expect(await Bun.file(join(mobile, ".credentials/keep")).text()).toBe("existing");
      failSubmit = true;
      await expect(submitEas([`beans-v${v}`, platform], runner, async () => trust, async () => binary, mobile)).rejects.toThrow("submit failed");
      expect(await Bun.file(join(mobile, "eas.json")).text()).toBe(originalConfig);
      expect(await Bun.file(keyPath).exists()).toBe(false);
      failSubmit = false;
      submitted = false;
      await expect(submitEas([`beans-v${v}`, platform], runner, async () => trust, async () => new Uint8Array([1]), mobile)).rejects.toThrow("bytes differ");
      expect(submitted).toBe(false);
      const other = generateKeyPairSync("ed25519").publicKey.export({ type: "spki", format: "der" }).subarray(-32).toString("base64");
      await expect(submitEas([`beans-v${v}`, platform], runner, async () => other, async () => binary, mobile)).rejects.toThrow("signature");
    }
    delete process.env.BEANS_EXPO_OWNER;
    await expect(submitEas([`beans-v${v}`, "android"])).rejects.toThrow("Linked Expo");
  } finally {
    for (const [name, value] of Object.entries(old)) { if (value === undefined) delete process.env[name]; else process.env[name] = value; }
    await rm(directory, { recursive: true, force: true });
  }
});
