// Build signed/notarized Mac assets for the unified GitHub release. Never upload to upstream hosting.
import { spawn } from "node:child_process";
import { accessSync, constants, existsSync, statSync } from "node:fs";
import { mkdir, rm } from "node:fs/promises";
import { delimiter, join } from "node:path";
import { APP_NAME, buildApp, bundlePath, codesign, color, log, readVersion, ROOT } from "./app.ts";
import { generateAppcast } from "./generate-appcast.ts";
import { releasePrivateKey } from "./release-signing.ts";
import { releaseProtocol } from "./release-github.ts";

type RunOptions = { cwd?: string; env?: NodeJS.ProcessEnv; capture?: boolean }

/** Run a command without a shell and report its exit code, so a failing step can be inspected. */
function spawnCommand(command: string[], options: RunOptions = {}): Promise<{ code: number; stdout: string }> {
  return new Promise((settle, reject) => {
    const child = spawn(command[0]!, command.slice(1), {
      cwd: options.cwd,
      env: options.env,
      shell: false,
      stdio: options.capture ? ["ignore", "pipe", "inherit"] : "inherit",
    });
    const chunks: Buffer[] = [];
    child.stdout?.on("data", (chunk: Buffer) => chunks.push(chunk));
    child.once("error", reject);
    child.once("close", (code, signal) => {
      if (code === null) {
        reject(new Error(`${command[0]} was terminated by signal ${signal ?? "unknown"}`));
        return;
      }
      settle({ code, stdout: Buffer.concat(chunks).toString("utf8") });
    });
  });
}

/** Run a command and fail on a nonzero exit, the way the Bun shell did. */
async function run(command: string[], options: RunOptions = {}): Promise<string> {
  const result = await spawnCommand(command, options);
  if (result.code !== 0) throw new Error(`${command[0]} failed with exit code ${result.code}`);
  return result.stdout;
}

/** Locate an executable on PATH. */
function which(command: string): string | null {
  const extensions = process.platform === "win32" ? (process.env.PATHEXT ?? ".EXE;.CMD;.BAT").split(";") : [""];
  for (const directory of (process.env.PATH ?? "").split(delimiter)) {
    for (const extension of extensions) {
      const candidate = join(directory, command + extension);
      try {
        if (statSync(candidate).isFile() && accessSync(candidate, constants.X_OK)) return candidate;
      } catch {
        continue;
      }
    }
  }
  return null;
}

const args = process.argv.slice(2);
if (args.some((arg) => arg !== "--local")) throw new Error("usage: pnpm run release-mac [--local]; set version in source before releasing");
const version = readVersion();
if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version)) throw new Error("Release version must be stable x.y.z");
await releasePrivateKey();
for (const tool of ["cargo", "swift", "ditto", "plutil", "xcrun", "create-dmg"]) {
  if (!which(tool)) throw new Error(`Missing required release tool: ${tool}`);
}
if (process.env.BEANS_DEFAULT_RELAY_URL?.trim()) {
  throw new Error("Public GitHub releases must not embed private relay metadata; unset BEANS_DEFAULT_RELAY_URL");
}
const identity = process.env.SIGN_IDENTITY ?? "Developer ID Application";
const notaryProfile = process.env.NOTARY_PROFILE ?? "BEANS_NOTARY";
const directory = join(ROOT, "dist", "mac");
const updates = join(directory, "updates");
const dmg = join(directory, `${APP_NAME}-${version}.dmg`);
const archive = join(updates, `${APP_NAME}-${version}.zip`);
const app = bundlePath("release");
await mkdir(directory, { recursive: true });
await rm(app, { recursive: true, force: true });
const build = await buildApp("release", { signIdentity: identity, onStep: (step) => log(color.dim(step)) });
if (!build.ok) throw new Error("Mac release build failed");
const plist = join(app, "Contents", "Info.plist");
const built = (await run(["plutil", "-extract", "CFBundleShortVersionString", "raw", plist], { capture: true })).trim();
if (built !== version) throw new Error(`Built Mac version ${built} does not match ${version}`);
const staging = join(directory, "dmg");
await rm(staging, { recursive: true, force: true });
await mkdir(staging, { recursive: true });
await run(["ditto", app, join(staging, `${APP_NAME}.app`)]);
await rm(dmg, { force: true });
const packed = await spawnCommand(["create-dmg", "--volname", `${APP_NAME} ${version}`, "--window-size", "540", "380", "--icon-size", "128", "--icon", `${APP_NAME}.app`, "150", "195", "--app-drop-link", "390", "195", "--hide-extension", `${APP_NAME}.app`, "--no-internet-enable", dmg, staging]);
if (!existsSync(dmg)) throw new Error("Disk image creation failed");
if (packed.code !== 0) log(color.yellow("Finder customization failed; verifying the produced disk image through signing and notarization"));
if (!await codesign(["--force", "--timestamp", "--sign", identity, dmg])) throw new Error("Disk image signing failed");
await run(["xcrun", "notarytool", "submit", dmg, "--keychain-profile", notaryProfile, "--wait"]);
await run(["xcrun", "stapler", "staple", dmg]);
await run(["xcrun", "stapler", "staple", app]);
await run(["codesign", "--verify", "--deep", "--strict", app]);
await run(["spctl", "--assess", "--type", "execute", app]);
await rm(updates, { recursive: true, force: true });
await mkdir(updates, { recursive: true });
await run(["ditto", "-c", "-k", "--keepParent", app, archive]);
const downloads = `https://github.com/bloodf/beans/releases/download/beans-v${version}/`;
if (!await generateAppcast(updates, downloads, await releaseProtocol())) throw new Error("Signed appcast generation failed");
log(`${color.green("built")} Beans ${version}; no upload performed`);
console.log(`DMG ${dmg}\nUpdate ${archive}\nAppcast ${join(updates, "appcast.xml")}`);
