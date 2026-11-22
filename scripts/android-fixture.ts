// Android build-tools for the release fixtures that generate real APKs (aapt2, apksigner).
// CI installs build-tools;34.0.0 and must run them: with `CI` set a missing tool is a failure, not a
// skip, so a broken runner image is still caught. On a developer machine without the Android SDK the
// fixtures are skipped with this reason instead of turning the whole root `pnpm test` red (#596).
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

export const ANDROID_BUILD_TOOLS_VERSION = "34.0.0";

export function androidSdk(env: Record<string, string | undefined> = process.env): string {
  return env.ANDROID_HOME ?? env.ANDROID_SDK_ROOT ?? join(homedir(), "Library/Android/sdk");
}

export function androidBuildTools(env: Record<string, string | undefined> = process.env): { path: string; available: boolean; skip: boolean; reason: string } {
  const path = join(androidSdk(env), "build-tools", ANDROID_BUILD_TOOLS_VERSION);
  const available = existsSync(join(path, "aapt2"));
  const reason = `aapt2 fixture prerequisite missing: Android build-tools ${ANDROID_BUILD_TOOLS_VERSION}`;
  return { path, available, skip: !available && !env.CI, reason };
}
