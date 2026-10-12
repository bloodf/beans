import { expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { readNativeIdentity } from "./release-native-identity.ts";
import { stageLocalBuild } from "./release-eas.ts";
import { ROOT, readVersion } from "./app.ts";

// One pipeline over public test-only artifacts: two repo APK fixtures (hash-pinned) read by the real
// aapt2, and IPAs zipped here and read by the real unzip/plutil. Metadata only: no signer, team or
// entitlement trust, no execution of artifact content, no AAB fixture exists so none is claimed.
const fixtures = process.env.BEANS_PUBLIC_APK_FIXTURES ?? resolve(ROOT, "../../artifacts/issue253-trust-positive");
const pinned = { "valid-test-only.apk": "46c3d5752513e1a6b0d9bb8c9e97d1b7f2085da7be993ef62fc759886f917ab8", "wrong-identity-test-only.apk": "be6c50011e80596093af6c1e6f5e7b551e5025b3270104c2aa917b8480c67016" };
const sdk = process.env.ANDROID_HOME ?? process.env.ANDROID_SDK_ROOT ?? join(homedir(), "Library/Android/sdk");
const buildTools = existsSync(join(sdk, "build-tools")) ? Bun.spawnSync(["ls", join(sdk, "build-tools")]).stdout.toString().trim().split("\n").filter(Boolean).sort().map(v => join(sdk, "build-tools", v)).find(d => existsSync(join(d, "aapt2"))) : undefined;
const env = { PATH: `${buildTools ? `${buildTools}:` : ""}/usr/bin:/bin` };
const plist = (id: string, version = "1.0.14", build = "42") => `<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>${id}</string><key>CFBundleShortVersionString</key><string>${version}</string><key>CFBundleVersion</key><string>${build}</string></dict></plist>`;
async function zipped(directory: string, name: string, files: Record<string, string>) {
  const root = await mkdtemp(join(directory, "src-"));
  for (const [file, body] of Object.entries(files)) { await mkdir(join(root, file, ".."), { recursive: true }); await writeFile(join(root, file), body); }
  const output = join(directory, name);
  if (Bun.spawnSync(["zip", "-X", "-D", "-q", "-r", output, "."], { cwd: root }).exitCode !== 0) throw new Error("zip fixture failed");
  return output;
}

test("native identity reader: real public APK metadata, real IPA plists, every malformed shape refused", async () => {
  const dir = await mkdtemp(join(tmpdir(), "beans-native-"));
  try {
    const app = "Payload/Beans.app", appex = `${app}/PlugIns/Notify.appex`;
    const ipa = await zipped(dir, "a.ipa", { [`${app}/Info.plist`]: plist("ai.amoena.beans"), [`${appex}/Info.plist`]: plist("ai.amoena.beans.notify") });
    expect(readNativeIdentity(ipa, "testflight", env)).toEqual({ identifier: "ai.amoena.beans", version: "1.0.14", buildNumber: "42" });
    const refusals: [string, () => unknown][] = [
      ["ipa without app plist", async () => readNativeIdentity(await zipped(dir, "n.ipa", { "Payload/x.txt": "x" }), "testflight", env)],
      ["ipa without extension", async () => readNativeIdentity(await zipped(dir, "e.ipa", { [`${app}/Info.plist`]: plist("ai.amoena.beans") }), "testflight", env)],
      ["ipa wrong extension id", async () => readNativeIdentity(await zipped(dir, "w.ipa", { [`${app}/Info.plist`]: plist("ai.amoena.beans"), [`${appex}/Info.plist`]: plist("ai.amoena.other") }), "testflight", env)],
      ["ipa extension number differs", async () => readNativeIdentity(await zipped(dir, "d.ipa", { [`${app}/Info.plist`]: plist("ai.amoena.beans"), [`${appex}/Info.plist`]: plist("ai.amoena.beans.notify", "1.0.14", "43") }), "testflight", env)],
      ["ipa malformed plist", async () => readNativeIdentity(await zipped(dir, "m.ipa", { [`${app}/Info.plist`]: "not a plist", [`${appex}/Info.plist`]: plist("ai.amoena.beans.notify") }), "testflight", env)],
      ["ipa missing version", async () => readNativeIdentity(await zipped(dir, "v.ipa", { [`${app}/Info.plist`]: plist("ai.amoena.beans").replace("<key>CFBundleShortVersionString</key><string>1.0.14</string>", ""), [`${appex}/Info.plist`]: plist("ai.amoena.beans.notify") }), "testflight", env)],
      ["PK magic is not an archive", async () => { await writeFile(join(dir, "pk.apk"), "PK\u0003\u0004 this is not a zip archive at all"); return readNativeIdentity(join(dir, "pk.apk"), "github", env); }],
      ["apk without manifest", async () => readNativeIdentity(await zipped(dir, "n.apk", { "classes.dex": "d" }), "github", env)],
      ["aab without base module", async () => readNativeIdentity(await zipped(dir, "n.aab", { "feature/manifest/AndroidManifest.xml": "x" }), "production", env)],
      ["aab tool unavailable is a refusal, not a pass", async () => readNativeIdentity(await zipped(dir, "b.aab", { "base/manifest/AndroidManifest.xml": "x" }), "production", { PATH: "/usr/bin:/bin" })],
      ["tool missing", async () => readNativeIdentity(await zipped(dir, "t.apk", { "AndroidManifest.xml": "x" }), "github", { PATH: "/nonexistent" })],
      ["relative path", () => readNativeIdentity("a.apk", "github", env)],
      ["unknown profile", () => readNativeIdentity(ipa, "preview" as any, env)],
      ["directory", () => readNativeIdentity(dir, "github", env)],
    ];
    for (const [name, call] of refusals) await expect(Promise.resolve().then(call), name).rejects.toThrow();
    // Staging consumes the reader: a number comes only from the binary and the proof carries no EAS id.
    const old = process.env.BEANS_EXPO_PROJECT_ID;
    process.env.BEANS_EXPO_PROJECT_ID = "11111111-1111-4111-8111-111111111111";
    try {
      const out = join(dir, "staged");
      await stageLocalBuild(ipa, "testflight", "a".repeat(40), out, async (p, q) => readNativeIdentity(p, q, env));
      const proof = await Bun.file(join(out, "eas-testflight.json")).json();
      expect(proof).toMatchObject({ kind: "local-exact-source", buildNumber: "42", version: readVersion() });
      expect(proof.buildId).toBeUndefined();
      // A real reading that differs from the release identity is refused and writes nothing.
      const other = await zipped(dir, "o.ipa", { [`${app}/Info.plist`]: plist("ai.amoena.beans", "9.9.9"), [`${appex}/Info.plist`]: plist("ai.amoena.beans.notify", "9.9.9") });
      await expect(stageLocalBuild(other, "testflight", "a".repeat(40), join(dir, "other"), async (p, q) => readNativeIdentity(p, q, env))).rejects.toThrow("differs from the release identity");
      expect(existsSync(join(dir, "other", "eas-testflight.json"))).toBe(false);
    } finally { if (old === undefined) delete process.env.BEANS_EXPO_PROJECT_ID; else process.env.BEANS_EXPO_PROJECT_ID = old; }
    // Real aapt2 on the public test-only APKs, only when both the pinned fixtures and the tool are present.
    const present = buildTools && Object.keys(pinned).every(name => existsSync(join(fixtures, name)));
    if (present) {
      for (const [name, digest] of Object.entries(pinned)) expect(createHash("sha256").update(await Bun.file(join(fixtures, name)).bytes()).digest("hex")).toBe(digest);
      expect(readNativeIdentity(join(fixtures, "valid-test-only.apk"), "github", env)).toEqual({ identifier: "ai.amoena.beans", version: "1.0.14", buildNumber: "42" });
      expect(readNativeIdentity(join(fixtures, "wrong-identity-test-only.apk"), "github", env).identifier).toBe("ai.amoena.beans.dev");
    }
    console.log(JSON.stringify({ realApkFixturesRead: Boolean(present) }));
  } finally { await rm(dir, { recursive: true, force: true }); }
});
