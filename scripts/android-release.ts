// Build the Rust phone core and a production, release-signed Android APK.
// Signing material stays outside the repository (BEANS_ANDROID_ENV overrides its location).
import { copyFile, mkdir, stat } from "node:fs/promises";
import { homedir } from "node:os";
import { join, resolve } from "node:path";
import { ROOT, readVersion } from "./app.ts";

const mobile = join(ROOT, "mobile");
const envPath = resolve((process.env.BEANS_ANDROID_ENV ?? "~/.config/beans/android/keystore.env").replace(/^~(?=\/)/, homedir()));
const env = { ...process.env } as Record<string, string>;
const names = ["BEANS_ANDROID_KEYSTORE", "BEANS_ANDROID_KEYSTORE_PASSWORD", "BEANS_ANDROID_KEY_ALIAS", "BEANS_ANDROID_KEY_PASSWORD"];

const file = Bun.file(envPath);
if (!(await file.exists())) throw new Error(`Missing signing environment: ${envPath}`);
if (((await stat(envPath)).mode & 0o777) !== 0o600) throw new Error(`Signing environment must have mode 0600: ${envPath}`);
const signing: Record<string, string> = {};
for (const line of (await file.text()).split(/\r?\n/)) {
  if (!line || line.startsWith("#")) continue;
  const match = /^([A-Z_]+)=(.*)$/.exec(line);
  if (!match || !names.includes(match[1]) || Object.hasOwn(signing, match[1])) throw new Error(`Invalid signing environment entry in ${envPath}`);
  signing[match[1]] = match[2];
}
for (const name of names) {
  if (!signing[name]) throw new Error(`Missing ${name} in ${envPath}`);
  env[name] = signing[name];
}
if (!(await Bun.file(env.BEANS_ANDROID_KEYSTORE).exists())) throw new Error(`Missing keystore: ${env.BEANS_ANDROID_KEYSTORE}`);
if (((await stat(env.BEANS_ANDROID_KEYSTORE)).mode & 0o777) !== 0o600) throw new Error(`Keystore must have mode 0600: ${env.BEANS_ANDROID_KEYSTORE}`);
delete env.LORCA_MOBILE_VARIANT;
delete env.EAS_BUILD_PROFILE;
// Homebrew cargo can be first on PATH, but Android target libraries live in rustup's toolchain.
const rustc = Bun.spawnSync(["rustup", "which", "rustc"], { stdout: "pipe", stderr: "pipe" });
if (rustc.exitCode === 0) env.RUSTC = rustc.stdout.toString().trim();

async function run(command: string[], cwd: string) {
  console.log(`$ ${command.join(" ")}`);
  const proc = Bun.spawn(command, { cwd, env, stdout: "inherit", stderr: "inherit" });
  if ((await proc.exited) !== 0) throw new Error(`${command[0]} failed with exit code ${proc.exitCode}`);
}

const started = Date.now();
await run(["bun", "run", "core", "android"], mobile);
await run(["bunx", "expo", "prebuild", "--clean", "--platform", "android", "--no-install"], mobile);
await run(["./gradlew", "app:assembleRelease"], join(mobile, "android"));
const apk = join(mobile, "android/app/build/outputs/apk/release/app-release.apk");
const size = (await stat(apk)).size;
const output = join(ROOT, "dist", "android", `Beans-${readVersion()}.apk`);
await mkdir(join(ROOT, "dist", "android"), { recursive: true });
await copyFile(apk, output);
console.log(`Signed APK: ${output} (${size} bytes, ${Math.round((Date.now() - started) / 1000)} s)`);
