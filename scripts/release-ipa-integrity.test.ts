import { expect, test } from "bun:test";
import { mkdir, mkdtemp, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { readVersion } from "./app.ts";
import { inspectMobileBinary } from "./release-build-inputs.ts";

test("native-valid unsigned IPA cannot substitute plist identity for signature integrity", async () => {
  const root = await mkdtemp(join(tmpdir(), "beans-ipa-fixture-"));
  const before = (await readdir(tmpdir())).filter(name => name.startsWith("beans-ipa-integrity-"));
  try {
    const version = readVersion();
    for (const [relative, identifier] of [["Payload/Beans.app", "ai.amoena.beans"], ["Payload/Beans.app/PlugIns/Notify.appex", "ai.amoena.beans.notify"]]) {
      const directory = join(root, relative);
      await mkdir(directory, { recursive: true });
      await writeFile(join(directory, "Info.plist"), `<?xml version="1.0"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>${identifier}</string><key>CFBundleShortVersionString</key><string>${version}</string><key>CFBundleVersion</key><string>45</string><key>CFBundleExecutable</key><string>Unsigned</string><key>CFBundlePackageType</key><string>APPL</string></dict></plist>`);
      await writeFile(join(directory, "Unsigned"), "Not signed code.\n");
    }
    const path = join(root, "unsigned-test-only.ipa");
    const zip = Bun.spawnSync(["zip", "-q", "-r", path, "Payload"], { cwd: root, stdout: "pipe", stderr: "pipe" });
    if (zip.exitCode) throw new Error("IPA fixture ZIP failed");
    expect(() => inspectMobileBinary(path, "testflight", version, "45", { profile: "testflight", teamId: "TESTONLY00", certificateSha256: ["0".repeat(64)] })).toThrow("Binary inspection tool failed: /usr/bin/codesign");
    expect((await readdir(tmpdir())).filter(name => name.startsWith("beans-ipa-integrity-")).sort()).toEqual(before.sort());
    console.log(JSON.stringify({ profile: "testflight", nativeIdentity: "matches", actualCodesign: "rejects-unsigned-extension", temporaryExtraction: "removed", appleTrust: "unsupported", positiveFixture: "unavailable" }));
  } finally { await rm(root, { recursive: true, force: true }); }
});
