import { expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import { chmod, mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { readNativeIdentity } from "./release-native-identity.ts";
import { stageLocalBuild } from "./release-eas.ts";
import { ROOT, readVersion } from "./app.ts";
import { archiveFixture } from "./release-archive-fixture.ts";
import { inspectReleaseArchive } from "./release-build-inputs.ts";

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
    // Real aapt2 on the public test-only APKs. Missing tool or fixtures fail the test: a skip is not a pass.
    if (!buildTools) throw new Error("aapt2 not found: install Android build-tools or set ANDROID_HOME");
    for (const [name, digest] of Object.entries(pinned)) {
      if (!existsSync(join(fixtures, name))) throw new Error(`Public APK fixture missing: ${name} (set BEANS_PUBLIC_APK_FIXTURES)`);
      expect(createHash("sha256").update(await Bun.file(join(fixtures, name)).bytes()).digest("hex")).toBe(digest);
    }
    expect(readNativeIdentity(join(fixtures, "valid-test-only.apk"), "github", env)).toEqual({ identifier: "ai.amoena.beans", version: "1.0.14", buildNumber: "42" });
    expect(readNativeIdentity(join(fixtures, "wrong-identity-test-only.apk"), "github", env)).toEqual({ identifier: "ai.amoena.beans.dev", version: "1.0.14", buildNumber: "42" });
  } finally { await rm(dir, { recursive: true, force: true }); }
});

// Stub tools record a spawn with the shell builtin printf, so they work with PATH limited to their own
// directory (touch would not be found there). The positive control proves a stub really leaves the marker,
// then clears only this scratch marker, so a later absence means no tool ran.
async function markerStubs(dir: string) {
  const marker = join(dir, "spawned"), bin = join(dir, "bin"); await mkdir(bin);
  const quoted = `'${marker.replaceAll("'", "'\\''")}'`;
  for (const tool of ["unzip", "aapt2", "bundletool", "plutil"]) { await writeFile(join(bin, tool), `#!/bin/sh\nprintf x > ${quoted}\nexit 0\n`); await chmod(join(bin, tool), 0o755); }
  for (const tool of ["unzip", "aapt2", "bundletool", "plutil"]) {
    expect(Bun.spawnSync([join(bin, tool)], { env: { PATH: bin } }).exitCode, `${tool} stub`).toBe(0);
    expect(existsSync(marker), `${tool} stub leaves the marker`).toBe(true);
    await rm(marker);
  }
  return { marker, bin };
}

// Hostile archives are refused by the shared inspector before any tool is spawned: stub tools on PATH
// would leave a marker if they ran. Duplicate names and span overlap are not claimed here (helper limits).
test("hostile archives are refused before any native tool spawns", async () => {
  const dir = await mkdtemp(join(tmpdir(), "beans-hostile-"));
  try {
    const { marker, bin } = await markerStubs(dir);
    const stubs = { PATH: bin };
    const central = (b: Buffer) => b.indexOf(Buffer.from([0x50, 0x4b, 1, 2]));
    const mutations: [string, string, (b: Buffer) => Buffer][] = [
      ["escape name", "Unsafe binary archive", () => archiveFixture("../escape")],
      ["absolute name", "Unsafe binary archive", () => archiveFixture("/absolute")],
      ["symlink entry", "Unsafe binary archive", () => archiveFixture("link", "../../escape", 0xa1ff)],
      ["oversize claim", "decompressed size", (b) => { b.writeUInt32LE(128 * 1024 * 1024 + 1, central(b) + 24); return b; }],
      ["size mismatch", "decompressed size", (b) => { b.writeUInt32LE(1, central(b) + 24); return b; }],
      ["encrypted flag", "Unsafe binary archive", (b) => { b.writeUInt16LE(1, central(b) + 8); return b; }],
      ["entry count mismatch", "Unsafe binary archive", (b) => { b.writeUInt16LE(2, b.lastIndexOf(Buffer.from([0x50, 0x4b, 5, 6])) + 10); return b; }],
    ];
    for (const [name, message, mutate] of mutations) {
      const path = join(dir, `${name.replaceAll(" ", "-")}.apk`);
      await writeFile(path, mutate(archiveFixture()));
      for (const profile of ["github", "production", "testflight"] as const) {
        expect(() => readNativeIdentity(path, profile, stubs), `${name} ${profile}`).toThrow(message);
      }
    }
    expect(existsSync(marker)).toBe(false);
  } finally { await rm(dir, { recursive: true, force: true }); }
});

// Nested overlap: the outer STORED member's payload embeds a complete inner local header, name and one
// byte, and the inner central entry points inside it. Every per-entry guard passes (distinct names,
// matching flags/method, in-bounds spans); only the final span comparison can refuse it.
function stored(entries: { name: string; local: number; size: number }[], body: Buffer): Buffer {
  const central = Buffer.concat(entries.map(({ name, local, size }) => {
    const header = Buffer.alloc(46 + name.length);
    header.writeUInt32LE(0x02014b50); header.writeUInt16LE(0x0314, 4); header.writeUInt16LE(20, 6);
    header.writeUInt32LE(size, 20); header.writeUInt32LE(size, 24); header.writeUInt16LE(name.length, 28);
    header.writeUInt32LE(local, 42); header.write(name, 46);
    return header;
  }));
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50); end.writeUInt16LE(entries.length, 8); end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(central.length, 12); end.writeUInt32LE(body.length, 16);
  return Buffer.concat([body, central, end]);
}
const localHeader = (name: string, size: number) => {
  const header = Buffer.alloc(30 + name.length);
  header.writeUInt32LE(0x04034b50); header.writeUInt16LE(20, 4);
  header.writeUInt32LE(size, 18); header.writeUInt32LE(size, 22); header.writeUInt16LE(name.length, 26); header.write(name, 30);
  return header;
};
test("nested span overlap is refused before any tool spawn; a non-overlapping two-entry control is admitted", async () => {
  const dir = await mkdtemp(join(tmpdir(), "beans-overlap-"));
  try {
    const { marker, bin } = await markerStubs(dir);
    // outer payload = inner local header + inner name + 1 data byte; inner offset points into it.
    const inner = Buffer.concat([localHeader("inner", 1), Buffer.from("x")]);
    const outerHeader = localHeader("outer", inner.length);
    const overlapping = stored([{ name: "outer", local: 0, size: inner.length }, { name: "inner", local: outerHeader.length, size: 1 }], Buffer.concat([outerHeader, inner]));
    expect(() => inspectReleaseArchive(overlapping)).toThrow("Unsafe binary archive");
    await writeFile(join(dir, "nested.apk"), overlapping);
    for (const profile of ["github", "production", "testflight"] as const) expect(() => readNativeIdentity(join(dir, "nested.apk"), profile, { PATH: bin }), profile).toThrow("Unsafe binary archive");
    expect(existsSync(marker)).toBe(false);
    // Control: the same two names laid out one after the other are admitted by the same inspector.
    const first = Buffer.concat([localHeader("outer", 1), Buffer.from("a")]), second = Buffer.concat([localHeader("inner", 1), Buffer.from("b")]);
    const control = stored([{ name: "outer", local: 0, size: 1 }, { name: "inner", local: first.length, size: 1 }], Buffer.concat([first, second]));
    expect(inspectReleaseArchive(control)).toEqual(["outer", "inner"]);
  } finally { await rm(dir, { recursive: true, force: true }); }
});

// The reader hands its tools a private snapshot, never the original path: a tool that swaps the original
// out from under the reader still sees the admitted bytes, and the snapshot is removed afterwards.
test("tools read the admitted snapshot even if the original is replaced; symlinks and growth are refused", async () => {
  const dir = await mkdtemp(join(tmpdir(), "beans-snapshot-"));
  try {
    const original = join(dir, "a.apk"), log = join(dir, "argv"), bin = join(dir, "bin"); await mkdir(bin);
    const zip = (tag: string) => archiveFixture("AndroidManifest.xml", tag);
    const admitted = zip("admitted"), replacement = zip("replaced");
    await writeFile(original, admitted);
    const q = (p: string) => `'${p.replaceAll("'", "'\\''")}'`;
    // unzip stub: record the path it received and its content hash, then replace the ORIGINAL file.
    await writeFile(join(bin, "unzip"), `#!/bin/sh\nfor last; do :; done\nprintf '%s ' "$last" >> ${q(log)}\nshasum -a 256 "$last" | cut -d' ' -f1 >> ${q(log)}\ncp ${q(join(dir, "replacement"))} ${q(original)}\nexit 0\n`);
    await writeFile(join(bin, "aapt2"), `#!/bin/sh\nfor last; do :; done\nprintf '%s ' "$last" >> ${q(log)}\nshasum -a 256 "$last" | cut -d' ' -f1 >> ${q(log)}\nprintf "package: name='ai.amoena.beans' versionCode='1' versionName='1.0.14'\\n"\n`);
    await writeFile(join(dir, "replacement"), replacement);
    for (const tool of ["unzip", "aapt2"]) await chmod(join(bin, tool), 0o755);
    const identity = readNativeIdentity(original, "github", { PATH: `${bin}:/usr/bin:/bin` });
    expect(identity).toEqual({ identifier: "ai.amoena.beans", version: "1.0.14", buildNumber: "1" });
    const rows = (await Bun.file(log).text()).trim().split("\n").map(line => line.split(" "));
    expect(rows.length).toBe(2);
    const admittedHash = createHash("sha256").update(admitted).digest("hex");
    for (const [path, hash] of rows) {
      expect(path).not.toBe(original);
      expect(hash).toBe(admittedHash);
      expect(existsSync(path)).toBe(false); // snapshot removed after the call
    }
    expect(createHash("sha256").update(await Bun.file(original).bytes()).digest("hex")).not.toBe(admittedHash); // original really was replaced
    // A symlink as the final component is refused before any tool runs.
    await writeFile(join(dir, "real.apk"), admitted);
    await Bun.spawn(["ln", "-s", join(dir, "real.apk"), join(dir, "link.apk")]).exited;
    expect(() => readNativeIdentity(join(dir, "link.apk"), "github", { PATH: bin })).toThrow();
    // Relative paths and non-regular files stay refused.
    expect(() => readNativeIdentity("a.apk", "github", { PATH: bin })).toThrow();
    expect(() => readNativeIdentity(dir, "github", { PATH: bin })).toThrow();
  } finally { await rm(dir, { recursive: true, force: true }); }
});

// A FIFO with no writer must be refused quickly, not wait forever in open(). The real snapshot and reader
// run in a subprocess under a watchdog; a watchdog kill is a failure, and no tool/inspector/file may appear.
test.skipIf(process.platform === "win32")("a FIFO without a writer is refused fast by the snapshot and the reader", async () => {
  const dir = await mkdtemp(join(tmpdir(), "beans-fifo-"));
  try {
    const fifo = join(dir, "pipe.apk"), marker = join(dir, "spawned"), bin = join(dir, "bin"), out = join(dir, "out");
    if (Bun.spawnSync(["mkfifo", fifo]).exitCode !== 0) throw new Error("mkfifo unavailable: the FIFO refusal cannot be checked on this platform");
    await mkdir(bin);
    const q = (p: string) => `'${p.replaceAll("'", "'\\''")}'`;
    for (const tool of ["unzip", "aapt2", "bundletool", "plutil"]) { await writeFile(join(bin, tool), `#!/bin/sh\nprintf x > ${q(marker)}\nexit 0\n`); await chmod(join(bin, tool), 0o755); }
    const script = join(dir, "probe.ts");
    await writeFile(script, `
      import { snapshotArtifact, readNativeIdentity } from ${JSON.stringify(join(ROOT, "scripts/release-native-identity.ts"))};
      import { stageLocalBuild } from ${JSON.stringify(join(ROOT, "scripts/release-eas.ts"))};
      const fifo = process.argv[2], bin = process.argv[3], out = process.argv[4];
      const refused = (run: () => unknown) => { try { run(); return "ACCEPTED"; } catch { return "refused"; } };
      process.env.BEANS_EXPO_PROJECT_ID = "11111111-1111-4111-8111-111111111111";
      let inspected = false;
      const staged = await stageLocalBuild(fifo, "github", "a".repeat(40), out, async () => { inspected = true; return { identifier: "ai.amoena.beans", version: "x", buildNumber: "1" }; }).then(() => "ACCEPTED", () => "refused");
      console.log(JSON.stringify({ snapshot: refused(() => snapshotArtifact(fifo)), reader: refused(() => readNativeIdentity(fifo, "github", { PATH: bin })), staged, inspected }));
    `);
    const started = Date.now();
    const child = Bun.spawn([process.execPath, script, fifo, bin, out], { stdout: "pipe", stderr: "pipe" });
    const watchdog = setTimeout(() => child.kill(), 10_000);
    const stdout = await new Response(child.stdout).text();
    const code = await child.exited;
    clearTimeout(watchdog);
    expect(Date.now() - started, "refusal must be fast, not a watchdog kill").toBeLessThan(10_000);
    expect(code, await new Response(child.stderr).text()).toBe(0);
    expect(JSON.parse(stdout.trim().split("\n").pop()!)).toEqual({ snapshot: "refused", reader: "refused", staged: "refused", inspected: false });
    expect(existsSync(marker)).toBe(false);
    expect(existsSync(out)).toBe(false);
  } finally { await rm(dir, { recursive: true, force: true }); }
});
