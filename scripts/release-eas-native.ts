import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdtemp, readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";
import { ROOT } from "./app.ts";

/** The pinned tsx loader, as an absolute file URL. The phone core build runs as its own Node
 * process with that loader, so it needs no `tsx` on PATH and no Bun runtime. */
function tsxLoader(): string {
  for (const base of [join(ROOT, "mobile", "package.json"), join(ROOT, "package.json")]) {
    try {
      const manifest = createRequire(base).resolve("tsx/package.json");
      const exports = JSON.parse(readFileSync(manifest, "utf8")).exports ?? {};
      const entry = typeof exports["."] === "string" ? exports["."] : "./dist/loader.mjs";
      return pathToFileURL(join(dirname(manifest), entry)).href;
    } catch {
      continue;
    }
  }
  throw new Error("tsx is not installed; install the dependencies before building the phone core");
}

// The package postinstall runs before Expo prebuild and CocoaPods. EAS's post-install hook runs after pods.
if (process.env.EAS_BUILD === "true") {
  const platform = process.env.EAS_BUILD_PLATFORM;
  if (platform !== "ios" && platform !== "android") throw new Error("EAS_BUILD_PLATFORM must be ios or android");
  async function run(command: string[]) {
    const child = spawn(command[0]!, command.slice(1), { cwd: ROOT, env: { ...process.env, PATH: `${process.env.HOME}/.cargo/bin:${process.env.PATH}` }, stdio: ["ignore", "inherit", "inherit"] });
    // Wait for the close: a spawn failure and a signalled child are failures, never a success.
    const [code, signal] = await new Promise<[number | null, NodeJS.Signals | null]>((settle, reject) => {
      child.once("error", reject);
      child.once("close", (exitCode, exitSignal) => settle([exitCode, exitSignal]));
    });
    if (code === null) throw new Error(`Native preparation failed: ${command[0]} (signal ${signal ?? "unknown"})`);
    if (code !== 0) throw new Error(`Native preparation failed: ${command[0]}`);
  }
  if (!existsSync(`${process.env.HOME}/.cargo/bin/rustup`)) {
    const directory = await mkdtemp(join(tmpdir(), "beans-rustup-"));
    const path = join(directory, "rustup.sh");
    await run(["curl", "--fail", "--proto", "=https", "--tlsv1.2", "https://sh.rustup.rs", "--output", path]);
    await run(["sh", path, "-y", "--profile", "minimal", "--default-toolchain", "stable"]);
  }
  await run(["rustup", "target", "add", ...(platform === "ios" ? ["aarch64-apple-ios", "aarch64-apple-ios-sim"] : ["aarch64-linux-android", "x86_64-linux-android"])]);
  if (platform === "android") await run(["cargo", "install", "cargo-ndk", "--locked"]);
  await run([process.execPath, "--import", tsxLoader(), join(ROOT, "mobile", "modules", "beans-core", "build.ts"), platform]);
}
