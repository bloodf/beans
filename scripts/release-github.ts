import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { copyFile, mkdir, readdir, realpath, stat, writeFile } from "node:fs/promises";
import { basename, join, resolve } from "node:path";
import { readVersion, ROOT } from "./app.ts";
import { extractReleaseNotes } from "./changelog.ts";
import { releasePrivateKey, signReleaseBytes } from "./release-signing.ts";
import { verifyEasBinary } from "./release-eas.ts";

export interface ReleaseArtifact {
  name: string;
  sha256: string;
  size: number;
  component: string;
  platform: string;
  version: string;
}
export interface ReleaseManifest {
  schema: 1;
  version: string;
  revision: string;
  protocol: number;
  artifacts: ReleaseArtifact[];
}
export type ReleaseScope = "server" | "all";

function releaseScope(value: string = "server"): ReleaseScope {
  if (value !== "server" && value !== "all") throw new Error("Release scope must be server or all");
  return value;
}
const repository = "bloodf/beans";
const stable = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const assetName = /^[A-Za-z0-9][A-Za-z0-9 ._-]{0,179}$/;
// GitHub normalizes spaces in the Windows installer filename when uploading.
function githubAssetName(name: string): string {
  return /^Beans Setup \d+\.\d+\.\d+\.exe$/.test(name) ? name.replaceAll(" ", ".") : name;
}

function releaseVersion(tag: string): string {
  const version = tag.replace(/^beans-v/, "");
  if (tag !== `beans-v${version}` || !stable.test(version) || version !== readVersion()) {
    throw new Error("Release tag must be beans-v<root package.json version>");
  }
  return version;
}
export async function releaseProtocol(): Promise<number> {
  const values = await Promise.all(["crates/cli/src/relay.rs", "crates/relay/src/routes.rs"].map(async (path) => {
    const match = (await Bun.file(join(ROOT, path)).text()).match(/^pub const PROTOCOL: u32 = (\d+);$/m);
    if (!match) throw new Error(`No protocol constant in ${path}`);
    return Number(match[1]);
  }));
  if (values[0] !== values[1] || values[0] < 5) throw new Error("Client and relay release protocols must match and be at least 5");
  return values[0];
}
async function digest(path: string): Promise<string> {
  const hash = createHash("sha256");
  for await (const bytes of createReadStream(path)) hash.update(bytes);
  return hash.digest("hex");
}
export async function coreVersion(): Promise<string> {
  const source = await Bun.file(join(ROOT, "Cargo.toml")).text();
  const version = source.match(/\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m)?.[1];
  if (!version || !stable.test(version)) throw new Error("Cargo workspace requires a stable version");
  return version;
}
function classify(name: string, version: string, core: string): Omit<ReleaseArtifact, "name" | "size" | "sha256"> {
  if (name === "beans-server-updater.py") return { component: "updater", platform: "linux", version };
  const server = name.match(/^beans-server-(linux-(?:x86_64|aarch64))\.tar\.gz$/);
  if (server) return { component: "server", platform: server[1], version: core };
  const cli = name.match(/^beans-cli-((?:macos-aarch64|linux-aarch64|linux-x86_64)\.tar\.gz|windows-x86_64\.zip)(\.sha256)?$/);
  if (cli) return { component: cli[2] ? "checksum" : "cli", platform: cli[1].replace(/\.(?:tar\.gz|zip)$/, ""), version: core };
  if (name === `Beans-${version}.apk`) return { component: "android", platform: "android", version };
  if (name === `Beans-${version}.ipa`) return { component: "ios", platform: "ios", version };
  if (name === `Beans-${version}-store.ipa`) return { component: "ios-store", platform: "ios", version };
  if (name === `Beans-${version}.aab`) return { component: "android-store", platform: "android", version };
  if (/^eas-(github|production|testflight)\.json$/.test(name)) return { component: "provenance", platform: "mobile", version };
  if ([`Beans-${version}.zip`, `Beans-${version}.dmg`, "appcast.xml"].includes(name)) return { component: "macos", platform: "macos-aarch64", version };
  const desktop = name.match(new RegExp(`^(?:update-(windows-amd64|linux-amd64|linux-arm64)\\.json|beans-${version.replaceAll(".", "\\.")}-(windows-amd64|linux-amd64|linux-arm64)\\.tar\\.gz|install-(linux-amd64|linux-arm64)\\.sh)$`));
  if (desktop) return { component: "desktop", platform: desktop[1] ?? desktop[2] ?? desktop[3], version };
  if (name === `Beans Setup ${version}.exe`) return { component: "desktop", platform: "windows-amd64", version };
  const deb = name.match(new RegExp(`^beans_${version.replaceAll(".", "\\.")}_(amd64|arm64)\\.deb$`));
  if (deb) return { component: "desktop", platform: `linux-${deb[1]}`, version };
  throw new Error(`Unexpected release asset: ${name}`);
}
function required(version: string, scope: ReleaseScope): string[] {
  const server = ["beans-server-linux-x86_64.tar.gz", "beans-server-linux-aarch64.tar.gz", "beans-server-updater.py"];
  if (scope === "server") return server;
  return [
    ...server,
    ...["macos-aarch64.tar.gz", "linux-aarch64.tar.gz", "linux-x86_64.tar.gz", "windows-x86_64.zip"].flatMap((suffix) => [`beans-cli-${suffix}`, `beans-cli-${suffix}.sha256`]),
    `Beans-${version}.zip`, `Beans-${version}.dmg`, "appcast.xml", `Beans-${version}.apk`, `Beans-${version}.aab`, `Beans-${version}-store.ipa`,
    "eas-github.json", "eas-production.json", "eas-testflight.json",
    ...["windows-amd64", "linux-amd64", "linux-arm64"].flatMap((platform) => [`update-${platform}.json`, `beans-${version}-${platform}.tar.gz`]),
    `Beans Setup ${version}.exe`, `beans_${version}_amd64.deb`, `beans_${version}_arm64.deb`, "install-linux-amd64.sh", "install-linux-arm64.sh",
  ];
}

function scopedArtifact(name: string, version: string, core: string, scope: ReleaseScope): Omit<ReleaseArtifact, "name" | "size" | "sha256"> {
  const artifact = classify(name, version, core);
  if (scope === "server" && !["server", "updater", "cli", "checksum"].includes(artifact.component)) {
    throw new Error(`Asset is outside server release scope: ${name}`);
  }
  return artifact;
}

export async function buildReleaseManifest(tag: string, revision: string, directory: string, scope: ReleaseScope = "server"): Promise<ReleaseManifest> {
  scope = releaseScope(scope);
  const version = releaseVersion(tag);
  if (!/^[a-f0-9]{40}$/.test(revision)) throw new Error("Release revision must be the exact40-character tag commit");
  const entries = await readdir(directory, { withFileTypes: true });
  const files = entries.filter((entry) => entry.isFile() && !["beans-update.json", "beans-update.json.sig"].includes(entry.name));
  for (const name of required(version, scope)) if (!files.some((file) => file.name === name)) throw new Error(`Release is incomplete: missing ${name}`);
  if (files.length > 200 || entries.some((entry) => !entry.isFile())) throw new Error("Release staging must be flat, regular files only");
  const core = await coreVersion();
  const artifacts = await Promise.all(files.sort((a, b) => a.name.localeCompare(b.name)).map(async ({ name }) => {
    if (!assetName.test(name) || basename(name) !== name) throw new Error("Unsafe release asset name");
    const path = join(directory, name);
    const size = (await stat(path)).size;
    if (!size || size > 2 ** 30) throw new Error(`Invalid release asset size: ${name}`);
    return { name, size, sha256: await digest(path), ...scopedArtifact(name, version, core, scope) };
  }));
  for (const artifact of artifacts) {
    if (artifact.component === "checksum" && !artifacts.some((archive) => archive.name === artifact.name.replace(/\.sha256$/, "") && archive.component === "cli")) {
      throw new Error(`CLI checksum has no archive: ${artifact.name}`);
    }
    if (artifact.component !== "cli") continue;
    const name = `${artifact.name}.sha256`;
    const checksum = artifacts.find((entry) => entry.name === name);
    if (!checksum || checksum.size > 1024) throw new Error(`Release is incomplete: missing or invalid ${name}`);
    const text = await Bun.file(join(directory, name)).text();
    if (text !== `${artifact.sha256}  ${artifact.name}\n` && text !== `${artifact.sha256} *${artifact.name}\n`) {
      throw new Error(`CLI checksum differs from archive: ${name}`);
    }
  }
  if (scope === "all") {
    const identities = new Set<string>();
    for (const profile of ["github", "production", "testflight"] as const) {
      const proof = await Bun.file(join(directory, `eas-${profile}.json`)).json();
      const name = `Beans-${version}${profile === "testflight" ? "-store.ipa" : profile === "production" ? ".aab" : ".apk"}`;
      const artifact = artifacts.find((entry) => entry.name === name)!;
      if (proof.schema !== 1 || proof.revision !== revision || proof.version !== version || proof.profile !== profile ||
          proof.artifact !== name || proof.sha256 !== artifact.sha256 || proof.size !== artifact.size ||
          !/^[a-f0-9-]{36}$/i.test(proof.project ?? "") || !/^[a-f0-9-]{36}$/i.test(proof.buildId ?? "") ||
          !/^[1-9]\d*$/.test(proof.buildNumber ?? "") || Number(proof.buildNumber) > 2_100_000_000 ||
          proof.platform !== (profile === "testflight" ? "IOS" : "ANDROID") || identities.has(proof.buildId)) {
        throw new Error(`Invalid EAS artifact provenance: ${profile}`);
      }
      identities.add(proof.buildId);
      const bytes = new Uint8Array(await Bun.file(join(directory, name)).arrayBuffer());
      if (createHash("sha256").update(bytes).digest("hex") !== artifact.sha256) throw new Error(`EAS binary changed during finalization: ${profile}`);
      await verifyEasBinary(bytes, profile, version, proof.buildNumber);
    }
  }
  return { schema: 1, version, revision, protocol: await releaseProtocol(), artifacts };
}
export async function writeReleaseManifest(tag: string, revision: string, directory: string, scope: ReleaseScope = "server"): Promise<ReleaseManifest> {
  const manifest = await buildReleaseManifest(tag, revision, directory, scope);
  const bytes = Buffer.from(`${JSON.stringify(manifest)}\n`);
  if (bytes.length > 64 * 1024) throw new Error("Release manifest exceeds64KiB");
  const signature = await signReleaseBytes(bytes);
  await writeFile(join(directory, "beans-update.json"), bytes);
  await writeFile(join(directory, "beans-update.json.sig"), `${signature}\n`);
  return manifest;
}

async function gh(args: string[]): Promise<string> {
  const process = Bun.spawn(["gh", ...args], { stdout: "pipe", stderr: "pipe" });
  const [stdout, stderr, code] = await Promise.all([new Response(process.stdout).text(), new Response(process.stderr).text(), process.exited]);
  if (code !== 0) throw new Error(`GitHub release operation failed (${code}): ${stderr}`);
  return stdout;
}
async function publish(tag: string, revision: string, directory: string, scope: ReleaseScope): Promise<void> {
  const manifest = await writeReleaseManifest(tag, revision, directory, scope);
  const release = JSON.parse(await gh(["api", `repos/${repository}/releases/tags/${tag}`]));
  if (release.draft || release.prerelease || release.tag_name !== tag) throw new Error("Only this exact published stable release can be finalized");
  const tagCommit = JSON.parse(await gh(["api", `repos/${repository}/commits/${tag}`])).sha;
  if (tagCommit !== revision) throw new Error("GitHub tag commit differs from the built release");
  // Preflight every immutable byte before uploading anything, including on scope changes.
  const expected = [
    ...manifest.artifacts,
    ...await Promise.all(["beans-update.json.sig", "beans-update.json"].map(async (name) => ({
      name, sha256: await digest(join(directory, name)), size: (await stat(join(directory, name))).size,
    }))),
  ];
  for (const existing of release.assets) {
    const artifact = expected.find((entry) => githubAssetName(entry.name) === existing.name);
    if (!artifact) throw new Error(`Unexpected existing release asset: ${existing.name}`);
    if (existing.digest !== `sha256:${artifact.sha256}` || existing.size !== artifact.size) {
      throw new Error(`Existing release asset differs: ${existing.name}`);
    }
  }
  if (release.assets.some((asset: { name: string }) => asset.name === "beans-update.json")) {
    if (expected.some((artifact) => !release.assets.some((asset: { name: string }) => asset.name === githubAssetName(artifact.name)))) {
      throw new Error("Finalized release is incomplete; immutable readiness cannot be extended or repaired");
    }
  }
  // Artifacts first, detached signature next, manifest last: readiness is per consumer.
  for (const artifact of expected) {
    if (artifact.name === "beans-update.json.sig") {
      const uploaded = JSON.parse(await gh(["api", `repos/${repository}/releases/tags/${tag}`]));
      for (const entry of manifest.artifacts) {
        const asset = uploaded.assets.find((asset: { name: string }) => asset.name === githubAssetName(entry.name));
        if (!asset || asset.size !== entry.size || asset.digest !== `sha256:${entry.sha256}`) {
          throw new Error(`Uploaded release asset differs: ${entry.name}`);
        }
      }
    }
    if (!release.assets.some((asset: { name: string }) => asset.name === githubAssetName(artifact.name))) {
      await gh(["release", "upload", tag, join(directory, artifact.name), "--repo", repository]);
    }
  }
  console.log(`Finalized signed GitHub release ${tag} at ${revision}`);
}

async function collect(source: string, destination: string, scope: ReleaseScope): Promise<void> {
  const version = readVersion();
  const core = await coreVersion();
  await mkdir(destination, { recursive: true });
  async function walk(path: string, platform: string): Promise<void> {
    for (const entry of await readdir(path, { withFileTypes: true })) {
      const current = join(path, entry.name);
      if (entry.isDirectory()) { await walk(current, entry.name); continue; }
      if (!entry.isFile()) throw new Error("Artifact staging cannot contain links");
      const name = entry.name === "install.sh" ? `install-${platform}.sh` : entry.name;
      if (!assetName.test(name)) throw new Error("Unsafe collected asset name");
      scopedArtifact(name, version, core, scope);
      const output = join(destination, name);
      if (await Bun.file(output).exists()) {
        if (await digest(current) !== await digest(output)) throw new Error(`Conflicting artifact name: ${name}`);
      } else await copyFile(current, output);
    }
  }
  await walk(source, "linux");
}

export async function stageDesktopAssets(inventory: string, destination: string): Promise<void> {
  const value = await Bun.file(inventory).json();
  const version = readVersion();
  if (value.version !== version || value.tag !== `beans-v${version}` || !Array.isArray(value.assets) || !value.assets.length) {
    throw new Error("Desktop staging inventory does not match this release");
  }
  await mkdir(destination, { recursive: true });
  for (const asset of value.assets) {
    if (typeof asset !== "string" || !/^desktop\/build\/(?:linux-amd64|linux-arm64|windows-amd64)\/[^/]+$/.test(asset)) {
      throw new Error("Unsafe desktop staging path");
    }
    const source = join(ROOT, asset);
    if (await realpath(source) !== source || !(await stat(source)).isFile()) throw new Error("Desktop asset is not a regular unlinked file");
    const target = asset.split("/")[2];
    const name = basename(asset) === "install.sh" ? `install-${target}.sh` : basename(asset);
    classify(name, version, await coreVersion());
    const output = join(destination, name);
    if (await Bun.file(output).exists()) throw new Error(`Duplicate staged desktop asset: ${name}`);
    await copyFile(source, output);
  }
}


if (import.meta.main) {
  const [command, first, second, third, fourth, ...extra] = process.argv.slice(2);
  if (extra.length) throw new Error("Unexpected release arguments");
  if (command === "check" && first && !third && !fourth) {
    const scope = releaseScope(second);
    const version = releaseVersion(first);
    if (scope === "all" && !extractReleaseNotes(await Bun.file(join(ROOT, "CHANGELOG.md")).text(), version)) {
      throw new Error(`CHANGELOG.md requires notes for ${version} before all-platform builds`);
    }
    await releasePrivateKey();
    console.log(`Release ${version}; scope ${scope}; protocol ${await releaseProtocol()}; signing key matches installed trust anchor`);
  } else if (command === "collect" && first && second && !fourth) {
    await collect(resolve(first), resolve(second), releaseScope(third));
  } else if (command === "desktop" && first && second && !third && !fourth) {
    await stageDesktopAssets(resolve(first), resolve(second));
  } else if (command === "manifest" && first && second && third) {
    await writeReleaseManifest(first, second, resolve(third), releaseScope(fourth));
  } else if (command === "publish" && first && second && third) {
    await publish(first, second, resolve(third), releaseScope(fourth));
  } else throw new Error("usage: release-github.ts check <tag> [server|all] | collect <source> <dir> [server|all] | desktop <inventory> <dir> | manifest|publish <tag> <revision> <dir> [server|all]");
}
