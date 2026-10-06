import { existsSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { ROOT } from "./app.ts";

// npm postinstall runs before Expo prebuild and CocoaPods. EAS's post-install hook runs after pods.
if (process.env.EAS_BUILD === "true") {
  const platform = process.env.EAS_BUILD_PLATFORM;
  if (platform !== "ios" && platform !== "android") throw new Error("EAS_BUILD_PLATFORM must be ios or android");
  async function run(command: string[]) {
    const child = Bun.spawn(command, { cwd: ROOT, env: { ...process.env, PATH: `${process.env.HOME}/.cargo/bin:${process.env.PATH}` }, stdout: "inherit", stderr: "inherit" });
    if (await child.exited !== 0) throw new Error(`Native preparation failed: ${command[0]}`);
  }
  if (!existsSync(`${process.env.HOME}/.cargo/bin/rustup`)) {
    const directory = await mkdtemp(join(tmpdir(), "beans-rustup-"));
    const path = join(directory, "rustup.sh");
    await run(["curl", "--fail", "--proto", "=https", "--tlsv1.2", "https://sh.rustup.rs", "--output", path]);
    await run(["sh", path, "-y", "--profile", "minimal", "--default-toolchain", "stable"]);
  }
  await run(["rustup", "target", "add", ...(platform === "ios" ? ["aarch64-apple-ios", "aarch64-apple-ios-sim"] : ["aarch64-linux-android", "x86_64-linux-android"])]);
  if (platform === "android") await run(["cargo", "install", "cargo-ndk", "--locked"]);
  await run(["bun", "run", "mobile/modules/lorca-core/build.ts", platform]);
}
