import { createHash, createPublicKey, verify } from "node:crypto";
import { lstat, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { ROOT, readVersion } from "./app.ts";
import { releasePublicKey } from "./release-signing.ts";
import { downloadArtifact, validateEasBuild } from "./release-eas.ts";

async function command(args: string[], cwd = ROOT) {
  const child = Bun.spawn(args, { cwd, stdout: "pipe", stderr: "inherit" });
  const text = await new Response(child.stdout).text();
  if (await child.exited !== 0) throw new Error(`${args[0]} failed; submission stopped`);
  return text;
}
export async function submitEas(args: string[], commandRunner = command, publicKey = releasePublicKey, downloader = downloadArtifact, mobileDirectory = join(ROOT, "mobile")) {
  const [tag, platform, ...extra] = args;
  if (extra.length || tag !== `beans-v${readVersion()}` || !["android", "ios"].includes(platform)) throw new Error("usage: submit-eas.ts <beans-v<root-version>> <android|ios>");
  if (!process.env.EXPO_TOKEN || !process.env.BEANS_EXPO_PROJECT_ID || !process.env.BEANS_EXPO_OWNER) throw new Error("Linked Expo configuration and EXPO_TOKEN are required");
  const revision = (await commandRunner(["git", "rev-parse", "HEAD"])).trim();
  const release = JSON.parse(await commandRunner(["gh", "api", `repos/bloodf/beans/releases/tags/${tag}`]));
  if (release.draft || release.prerelease || release.tag_name !== tag || JSON.parse(await commandRunner(["gh", "api", `repos/bloodf/beans/commits/${tag}`])).sha !== revision) throw new Error("Submission requires exact published stable tag source");
  const directory = await mkdtemp(join(tmpdir(), "beans-submit-"));
  const profile = platform === "android" ? "production" : "testflight";
  const names = ["beans-update.json", "beans-update.json.sig", `eas-${profile}.json`];
  await commandRunner(["gh", "release", "download", tag, "--repo", "bloodf/beans", "--dir", directory, ...names.flatMap((name) => ["--pattern", name])]);
  const bytes = new Uint8Array(await Bun.file(join(directory, names[0])).arrayBuffer());
  if (bytes.length > 64 * 1024) throw new Error("Readiness manifest exceeds 64 KiB");
  const signature = (await Bun.file(join(directory, names[1])).text()).trim();
  const key = createPublicKey({ key: Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), Buffer.from(await publicKey(), "base64")]), format: "der", type: "spki" });
  if (!verify(null, bytes, key, Buffer.from(signature, "base64"))) throw new Error("Invalid Beans release signature");
  const manifest = JSON.parse(Buffer.from(bytes).toString());
  if (manifest.schema !== 1 || manifest.revision !== revision || manifest.version !== readVersion()) throw new Error("Signed release source differs");
  const proofBytes = new Uint8Array(await Bun.file(join(directory, names[2])).arrayBuffer());
  const entry = manifest.artifacts.find((asset: any) => asset.name === names[2]);
  if (!entry || entry.size !== proofBytes.length || entry.sha256 !== createHash("sha256").update(proofBytes).digest("hex")) throw new Error("EAS provenance differs from signed release");
  const proof = JSON.parse(Buffer.from(proofBytes).toString());
  const build = JSON.parse(await commandRunner(["eas", "build:view", proof.buildId, "--json"], mobileDirectory));
  validateEasBuild(build, profile, revision, readVersion(), process.env.BEANS_EXPO_PROJECT_ID);
  if (proof.buildNumber !== build.appBuildVersion || proof.profile !== profile || proof.project !== build.app.id) throw new Error("EAS build differs from signed provenance");
  const artifact = manifest.artifacts.find((asset: any) => asset.name === proof.artifact);
  if (!artifact || artifact.sha256 !== proof.sha256 || artifact.size !== proof.size) throw new Error("Store artifact is absent from signed inventory");
  const binary = await downloader(build.artifacts.buildUrl);
  if (binary.length !== artifact.size || binary.length > 2 ** 30 || createHash("sha256").update(binary).digest("hex") !== artifact.sha256) throw new Error("Store artifact bytes differ from signed inventory");
  const path = join(directory, platform === "android" ? "store.aab" : "store.ipa");
  await writeFile(path, binary, { flag: "wx" });
  const configPath = join(mobileDirectory, "eas.json");
  if (!(await lstat(configPath)).isFile()) throw new Error("EAS config must be a regular unlinked file");
  const original = await readFile(configPath);
  const config = JSON.parse(original.toString());
  const credentials = await mkdtemp(join(tmpdir(), "beans-submit-credentials-"));
  try {
    if (platform === "android") {
      const json = process.env.BEANS_PLAY_SERVICE_ACCOUNT_JSON;
      if (!json || JSON.parse(json).type !== "service_account") throw new Error("BEANS_PLAY_SERVICE_ACCOUNT_JSON must authorize Play Developer API");
      const keyPath = join(credentials, "play.json");
      await writeFile(keyPath, json, { mode: 0o600, flag: "wx" });
      if (config.submit.production.android.track !== "internal" || config.submit.production.android.releaseStatus !== "completed") throw new Error("Only Play internal testing submission is permitted");
      await writeFile(configPath, JSON.stringify({ ...config, submit: { ...config.submit, production: { ...config.submit.production, android: { ...config.submit.production.android, serviceAccountKeyPath: keyPath } } } }));
    } else {
      const { BEANS_ASC_APP_ID: appId, BEANS_ASC_KEY_ID: keyId, BEANS_ASC_ISSUER_ID: issuer, BEANS_ASC_KEY_P8: p8 } = process.env;
      if (!/^[1-9]\d*$/.test(appId ?? "") || !/^[A-Z0-9]{10}$/.test(keyId ?? "") || !/^[a-f0-9-]{36}$/i.test(issuer ?? "") || !p8?.includes("PRIVATE KEY")) throw new Error("TestFlight requires BEANS_ASC_APP_ID, BEANS_ASC_KEY_ID, BEANS_ASC_ISSUER_ID and BEANS_ASC_KEY_P8");
      const keyPath = join(credentials, "asc.p8");
      await writeFile(keyPath, p8, { mode: 0o600, flag: "wx" });
      await writeFile(configPath, JSON.stringify({ ...config, submit: { ...config.submit, production: { ...config.submit.production, ios: { ...config.submit.production.ios, ascApiKeyPath: keyPath, ascAppId: appId, ascApiKeyId: keyId, ascApiKeyIssuerId: issuer } } } }));
    }
    // Submit verified bytes, never --latest. iOS Submit uploads to TestFlight, not App Store promotion.
    await commandRunner(["eas", "submit", "--platform", platform, "--profile", "production", "--path", path, "--non-interactive", "--wait"], mobileDirectory);
  } finally {
    try { await writeFile(configPath, original); }
    finally { await rm(credentials, { recursive: true, force: true }); }
  }
}

if (import.meta.main) await submitEas(process.argv.slice(2));
