// The ONNX Runtime a Runner runs local ONNX and image models with, placed beside the CLI binary
// where `embeddings::assets::bundled_runtime` finds it. Each platform's library comes from the
// official PyPI wheel pinned in crates/cli/onnxruntime.json: downloaded once into a cache,
// checked against its size and SHA-256, and unzipped; the library is checked against its own
// pin, which a Runner with no runtime beside it also uses to download the same wheel with the
// user's approval (`embeddings/runtime_download.rs`). Models are never bundled.
//
//   tsx scripts/onnxruntime.ts                      # the host's runtime, path printed
//   tsx scripts/onnxruntime.ts darwin-arm64 out/bin # one target into a directory

import { createHash } from "node:crypto"
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs"
import { homedir } from "node:os"
import { dirname, join } from "node:path"
import { inflateRawSync } from "node:zlib"

const ROOT = join(import.meta.dirname, "..")
const MANIFEST = join(ROOT, "crates", "cli", "onnxruntime.json")

type Entry = { url: string; bytes: number; sha256: string; member: string; library_bytes: number; library_sha256: string }
type Manifest = { version: string; targets: Record<string, Entry> }

/** The library's name beside the binary, as `AssetKind::Runtime::filename` expects it. */
export function runtimeFile(target: string): string {
  if (target.startsWith("darwin")) return "libonnxruntime.dylib"
  if (target.startsWith("windows")) return "onnxruntime.dll"
  return "libonnxruntime.so"
}

/** A manifest target for a desktop platform (`darwin/arm64`) or Node's host names. */
export function targetFor(os: string, arch: string): string {
  const system = os === "win32" ? "windows" : os
  const cpu = arch === "amd64" || arch === "x64" ? "x64" : arch
  return `${system}-${cpu}`
}

export function manifest(): Manifest {
  return JSON.parse(readFileSync(MANIFEST, "utf8")) as Manifest
}

/** Whether a runtime is pinned for `target` (no official wheel exists for some, as Intel Macs). */
export function isPinned(target: string): boolean {
  return Object.hasOwn(manifest().targets, target)
}

/** One stored or deflated file from a zip archive (a wheel), by its path inside it. */
export function extractMember(zip: Buffer, name: string): Buffer {
  // The end-of-central-directory record: its signature, within the last 64 KiB + 22 bytes.
  let end = -1
  for (let i = zip.length - 22; i >= Math.max(0, zip.length - 65_557); i--) {
    if (zip.readUInt32LE(i) === 0x06054b50) {
      end = i
      break
    }
  }
  if (end < 0) throw new Error("not a zip archive")
  const count = zip.readUInt16LE(end + 10)
  let at = zip.readUInt32LE(end + 16)
  for (let entry = 0; entry < count; entry++) {
    if (at + 46 > end || zip.readUInt32LE(at) !== 0x02014b50) throw new Error("a damaged zip directory")
    const method = zip.readUInt16LE(at + 10)
    const compressed = zip.readUInt32LE(at + 20)
    const size = zip.readUInt32LE(at + 24)
    const nameLength = zip.readUInt16LE(at + 28)
    const extraLength = zip.readUInt16LE(at + 30)
    const commentLength = zip.readUInt16LE(at + 32)
    const local = zip.readUInt32LE(at + 42)
    const entryName = zip.toString("utf8", at + 46, at + 46 + nameLength)
    at += 46 + nameLength + extraLength + commentLength
    if (entryName !== name) continue
    if (local + 30 > end || zip.readUInt32LE(local) !== 0x04034b50) throw new Error("a damaged zip entry")
    const start = local + 30 + zip.readUInt16LE(local + 26) + zip.readUInt16LE(local + 28)
    if (start + compressed > end) throw new Error(`${name} runs past the archive`)
    const data = zip.subarray(start, start + compressed)
    const out = method === 0 ? Buffer.from(data) : method === 8 ? inflateRawSync(data) : null
    if (!out) throw new Error(`${name} uses zip method ${method}`)
    if (out.length !== size) throw new Error(`${name} unpacked to ${out.length} bytes, not ${size}`)
    return out
  }
  throw new Error(`${name} is not in the archive`)
}

function sha256(bytes: Buffer): string {
  return createHash("sha256").update(bytes).digest("hex")
}

/** The pinned wheel for `target`, from the cache or downloaded into it, checked. */
async function wheel(target: string, cache: string): Promise<{ bytes: Buffer; entry: Entry }> {
  const entry = isPinned(target) ? manifest().targets[target] : undefined
  if (!entry) throw new Error(`no ONNX Runtime is pinned for ${target}`)
  const cached = join(cache, `${entry.sha256}.whl`)
  if (existsSync(cached)) {
    const bytes = readFileSync(cached)
    if (bytes.length === entry.bytes && sha256(bytes) === entry.sha256) return { bytes, entry }
  }
  const response = await fetch(entry.url)
  if (!response.ok) throw new Error(`downloading the ONNX Runtime for ${target}: HTTP ${response.status}`)
  const bytes = Buffer.from(await response.arrayBuffer())
  if (bytes.length !== entry.bytes || sha256(bytes) !== entry.sha256) {
    throw new Error(`the ONNX Runtime wheel for ${target} is not the pinned one (${bytes.length} bytes, sha256 ${sha256(bytes)})`)
  }
  mkdirSync(cache, { recursive: true })
  writeFileSync(`${cached}.partial`, bytes)
  renameSync(`${cached}.partial`, cached)
  return { bytes, entry }
}

/** Places `target`'s runtime in `directory` and returns its path. */
export async function placeOnnxRuntime(target: string, directory: string, cache = join(homedir(), ".cache", "beans", "onnxruntime")): Promise<string> {
  const { bytes, entry } = await wheel(target, cache)
  const library = extractMember(bytes, entry.member)
  if (library.length !== entry.library_bytes || sha256(library) !== entry.library_sha256) {
    throw new Error(`the ONNX Runtime library for ${target} is not the pinned one (${library.length} bytes, sha256 ${sha256(library)})`)
  }
  const path = join(directory, runtimeFile(target))
  mkdirSync(dirname(path), { recursive: true })
  writeFileSync(path, library, { mode: 0o644 })
  return path
}

if (import.meta.main || process.argv[1]?.endsWith("onnxruntime.ts")) {
  const target = process.argv[2] ?? targetFor(process.platform, process.arch)
  const directory = process.argv[3] ?? join(ROOT, "target", "onnxruntime", target)
  placeOnnxRuntime(target, directory).then(
    (path) => console.log(path),
    (error: unknown) => {
      console.error(error instanceof Error ? error.message : error)
      process.exit(1)
    },
  )
}
