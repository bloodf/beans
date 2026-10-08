import { expect, spyOn, test } from "bun:test";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { readVersion } from "./app.ts";
import { stageEasBuild } from "./release-eas.ts";
import { buildReleaseManifest } from "./release-github.ts";

// Stored ZIP fixtures are structurally valid archives, not signed native binaries.
// No fixture certificate is an approved Beans signer. Unsigned candidates must fail closed.
function archive(files: Record<string, string>): Uint8Array {
  const records: Buffer[] = [], directory: Buffer[] = [];
  let offset = 0;
  for (const [path, text] of Object.entries(files)) {
    const name = Buffer.from(path), bytes = Buffer.from(text);
    let crc = 0xffffffff;
    for (const byte of bytes) {
      crc ^= byte;
      for (let bit = 0; bit < 8; bit++) crc = (crc >>> 1) ^ (crc & 1 ? 0xedb88320 : 0);
    }
    crc = (crc ^ 0xffffffff) >>> 0;
    const local = Buffer.alloc(30), central = Buffer.alloc(46);
    local.writeUInt32LE(0x04034b50); local.writeUInt16LE(20, 4);
    local.writeUInt32LE(crc, 14); local.writeUInt32LE(bytes.length, 18);
    local.writeUInt32LE(bytes.length, 22); local.writeUInt16LE(name.length, 26);
    central.writeUInt32LE(0x02014b50); central.writeUInt16LE(20, 4); central.writeUInt16LE(20, 6);
    central.writeUInt32LE(crc, 16); central.writeUInt32LE(bytes.length, 20);
    central.writeUInt32LE(bytes.length, 24); central.writeUInt16LE(name.length, 28);
    central.writeUInt32LE(offset, 42);
    records.push(local, name, bytes); directory.push(central, name);
    offset += local.length + name.length + bytes.length;
  }
  const index = Buffer.concat(directory), end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50); end.writeUInt16LE(directory.length / 2, 8);
  end.writeUInt16LE(directory.length / 2, 10); end.writeUInt32LE(index.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...records, index, end]);
}
const hash = (bytes: Uint8Array | string) => createHash("sha256").update(bytes).digest("hex");

// Intentionally red until the existing staging boundary performs native inspection.
// Exercise production public functions, not a second fixture-only verifier.
test("binary assurance rejects native identity/module/unsigned candidates and stale provenance", async () => {
  const root = await mkdtemp(join(tmpdir(), "beans-binary-assurance-"));
  const version = readVersion(), revision = "a".repeat(40);
  const project = "11111111-1111-4111-8111-111111111111";
  const previous = process.env.BEANS_EXPO_PROJECT_ID;
  process.env.BEANS_EXPO_PROJECT_ID = project;
  const build = { id: "22222222-2222-4222-8222-222222222222", status: "FINISHED", platform: "ANDROID", appIdentifier: "ai.amoena.beans", distribution: "STORE", buildProfile: "github", gitCommitHash: revision, appVersion: version, appBuildVersion: "42", app: { id: project }, artifacts: { buildUrl: "https://artifacts.eascdn.net/fixture.apk" } };
  let payload = archive({ "AndroidManifest.xml": "<manifest package='ai.amoena.beans'/>" });
  const fetcher = spyOn(globalThis, "fetch").mockImplementation(async () => new Response(payload) as any);
  const accepted: string[] = [];
  try {
    // Control: finished metadata alone still rejects an explicitly wrong EAS identity.
    await expect(stageEasBuild({ ...build, appIdentifier: "ai.amoena.beans.dev" }, "github", revision, join(root, "metadata-control"))).rejects.toThrow("EAS build");
    const cases = [
      { label: "wrong APK native identity", profile: "github", platform: "ANDROID", files: { "AndroidManifest.xml": "<manifest package='ai.amoena.beans.dev' android:versionName='0.0.1' android:versionCode='1'/>" } },
      { label: "AAB missing base module", profile: "production", platform: "ANDROID", files: { "feature/manifest/AndroidManifest.xml": "<manifest package='ai.amoena.beans'/>" } },
      { label: "wrong IPA app and extension identity", profile: "testflight", platform: "IOS", files: { "Payload/Beans.app/Info.plist": "<plist><dict><key>CFBundleIdentifier</key><string>ai.amoena.beans.dev</string></dict></plist>", "Payload/Beans.app/PlugIns/Notify.appex/Info.plist": "<plist><dict><key>CFBundleIdentifier</key><string>foreign.notify</string></dict></plist>" } },
      { label: "unsigned APK signer candidate", profile: "github", platform: "ANDROID", files: { "AndroidManifest.xml": `<manifest package='ai.amoena.beans' android:versionName='${version}' android:versionCode='42'/>`, "META-INF/CANDIDATE.SF": "Signature-Version: 1.0\nUnsigned synthetic candidate; no certificate or signature.\n" } },
    ] as const;
    for (const [index, candidate] of cases.entries()) {
      payload = archive(candidate.files);
      const directory = join(root, String(index));
      try {
        await stageEasBuild({ ...build, platform: candidate.platform, buildProfile: candidate.profile }, candidate.profile, revision, directory);
        const proof = JSON.parse(await readFile(join(directory, `eas-${candidate.profile}.json`), "utf8"));
        if (proof.sha256 === hash(payload)) accepted.push(candidate.label);
        else throw new Error("Unexpected staging result: fixture bytes were not bound to provenance");
      } catch (error) {
        if (!(error instanceof Error) || !/binary|manifest|signature|signer|certificate|module|identity|verif|tool|provision/i.test(error.message)) throw error;
      }
    }
    // Real finalization boundary: stale digest cannot be repaired by matching EAS metadata.
    const inventory = join(root, "0"), apk = `Beans-${version}.apk`;
    const names = ["beans-server-linux-x86_64.tar.gz", "beans-server-linux-aarch64.tar.gz", "beans-server-updater.py", `Beans-${version}.zip`, `Beans-${version}.dmg`, "appcast.xml", `Beans-${version}.aab`, `Beans-${version}-store.ipa`, `Beans Setup ${version}.exe`, `beans_${version}_amd64.deb`, `beans_${version}_arm64.deb`, "install-linux-amd64.sh", "install-linux-arm64.sh", ...["windows-amd64", "linux-amd64", "linux-arm64"].flatMap(p => [`update-${p}.json`, `beans-${version}-${p}.tar.gz`])];
    // The inventory also exists when a future verifier correctly refuses every candidate.
    await mkdir(inventory, { recursive: true });
    for (const name of [...names, apk]) await writeFile(join(inventory, name), "fixture");
    for (const suffix of ["macos-aarch64.tar.gz", "linux-aarch64.tar.gz", "linux-x86_64.tar.gz", "windows-x86_64.zip"]) {
      const name = `beans-cli-${suffix}`;
      await writeFile(join(inventory, name), "fixture");
      await writeFile(join(inventory, `${name}.sha256`), `${hash("fixture")}  ${name}\n`);
    }
    for (const [index, profile] of ["github", "production", "testflight"].entries()) {
      const artifact = `Beans-${version}${profile === "github" ? ".apk" : profile === "production" ? ".aab" : "-store.ipa"}`;
      await writeFile(join(inventory, `eas-${profile}.json`), JSON.stringify({ schema: 1, version, revision, profile, artifact, size: 7, sha256: hash("fixture"), project, buildId: `${index + 2}2222222-2222-4222-8222-222222222222`, buildNumber: "42", platform: profile === "testflight" ? "IOS" : "ANDROID" }));
    }
    await writeFile(join(inventory, apk), "changed");
    await expect(buildReleaseManifest(`beans-v${version}`, revision, inventory, "all")).rejects.toThrow("Invalid EAS artifact provenance: github");
    expect(accepted, "Successful hash-bound provenance must not authorize these unverified native candidates").toEqual([]);
  } finally {
    fetcher.mockRestore();
    if (previous === undefined) delete process.env.BEANS_EXPO_PROJECT_ID; else process.env.BEANS_EXPO_PROJECT_ID = previous;
    await rm(root, { recursive: true, force: true });
  }
});
