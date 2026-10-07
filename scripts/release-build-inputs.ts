import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

export function hostLibrary(platform: string): string {
  if (platform === "darwin") return "liblorca_mobile.dylib";
  if (platform === "linux") return "liblorca_mobile.so";
  throw new Error("Mobile binding generation requires macOS or Linux host");
}
export function androidNdk(env: Record<string, string | undefined>, platform: string, exists = existsSync): string {
  const explicit = env.ANDROID_NDK_HOME ?? env.ANDROID_NDK_ROOT;
  const sdk = env.ANDROID_HOME ?? env.ANDROID_SDK_ROOT ?? (platform === "darwin" ? join(homedir(), "Library/Android/sdk") : undefined);
  if (env.BEANS_ANDROID_NDK_VERSION && !/^\d+\.\d+\.\d+$/.test(env.BEANS_ANDROID_NDK_VERSION)) throw new Error("Invalid BEANS_ANDROID_NDK_VERSION");
  const path = explicit ?? (sdk && env.BEANS_ANDROID_NDK_VERSION ? join(sdk, "ndk", env.BEANS_ANDROID_NDK_VERSION) : undefined);
  if (!path || !exists(join(path, "source.properties"))) throw new Error("Set ANDROID_NDK_HOME/ANDROID_NDK_ROOT or SDK root plus BEANS_ANDROID_NDK_VERSION to an installed NDK");
  return path;
}
