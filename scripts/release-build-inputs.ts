import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { inflateRawSync } from "node:zlib";

export function hostLibrary(platform: string): string {
  if (platform === "darwin") return "libbeans_mobile.dylib";
  if (platform === "linux") return "libbeans_mobile.so";
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

// ZIP64 is deliberately unsupported: release inspection has explicit finite ceilings.
export function inspectReleaseArchive(bytes: Uint8Array): string[] {
  const data = Buffer.from(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const fail = () => { throw new Error("Unsafe binary archive structure or decompressed size"); };
  if (data.length < 22 || data.length > 2 ** 30) fail();
  let end = data.length - 22;
  while (end >= Math.max(0, data.length - 65557) && data.readUInt32LE(end) !== 0x06054b50) end--;
  if (end < Math.max(0, data.length - 65557) || end + 22 + data.readUInt16LE(end + 20) !== data.length) fail();
  const count = data.readUInt16LE(end + 10), size = data.readUInt32LE(end + 12), start = data.readUInt32LE(end + 16);
  if (data.readUInt32LE(end + 4) !== 0 || data.readUInt16LE(end + 8) !== count || !count || count > 20000 || start + size !== end) fail();
  let cursor = start, total = 0;
  const names = new Set<string>(), spans: [number, number][] = [];
  for (let index = 0; index < count; index++) {
    if (cursor + 46 > end || data.readUInt32LE(cursor) !== 0x02014b50) fail();
    const flags = data.readUInt16LE(cursor + 8), method = data.readUInt16LE(cursor + 10);
    const packed = data.readUInt32LE(cursor + 20), unpacked = data.readUInt32LE(cursor + 24);
    const length = data.readUInt16LE(cursor + 28), extra = data.readUInt16LE(cursor + 30), comment = data.readUInt16LE(cursor + 32);
    const local = data.readUInt32LE(cursor + 42), mode = data.readUInt32LE(cursor + 38) >>> 16;
    if (cursor + 46 + length + extra + comment > end || flags & 1 || ![0, 8].includes(method) || data.readUInt16LE(cursor + 34) || (mode & 0xf000) === 0xa000 || unpacked > 128 * 1024 * 1024 || (total += unpacked) > 512 * 1024 * 1024) fail();
    const rawName = data.subarray(cursor + 46, cursor + 46 + length), name = rawName.toString("utf8");
    if (!length || !Buffer.from(name).equals(rawName) || names.has(name) || name.startsWith("/") || /^[A-Za-z]:/.test(name) || /[\\\x00-\x1f\x7f*?\[\]]/.test(name) || name.split("/").some(part => part === ".." || part === ".")) fail();
    if (local + 30 > start || data.readUInt32LE(local) !== 0x04034b50 || data.readUInt16LE(local + 6) !== flags || data.readUInt16LE(local + 8) !== method) fail();
    const localLength = data.readUInt16LE(local + 26), localExtra = data.readUInt16LE(local + 28), payload = local + 30 + localLength + localExtra;
    if (payload + packed > start || !data.subarray(local + 30, local + 30 + localLength).equals(rawName)) fail();
    const compressed = data.subarray(payload, payload + packed);
    const actual = method === 0 ? compressed : inflateRawSync(compressed, { maxOutputLength: Math.max(1, unpacked) });
    if (actual.length !== unpacked) fail();
    names.add(name); spans.push([local, payload + packed]);
    cursor += 46 + length + extra + comment;
  }
  spans.sort((a, b) => a[0] - b[0]);
  if (cursor !== end || spans.some((span, index) => index > 0 && span[0] < spans[index - 1][1])) fail();
  return [...names];
}
