import { deflateRawSync } from "node:zlib"
import { expect, test } from "vitest"
import { extractMember, isPinned, manifest, runtimeFile, targetFor } from "./onnxruntime.ts"

/** A zip of `files`, the first stored and the rest deflated, as a wheel holds them; `method`
 * overrides the second file's method as recorded (not how it is stored). */
function zip(files: [string, Buffer][], method?: number): Buffer {
  const locals: Buffer[] = []
  const centrals: Buffer[] = []
  let offset = 0
  files.forEach(([name, data], index) => {
    const stored = index === 0 ? 0 : 8
    const body = stored === 0 ? data : deflateRawSync(data)
    const recorded = index === 1 && method !== undefined ? method : stored
    const nameBytes = Buffer.from(name)
    const local = Buffer.alloc(30)
    local.writeUInt32LE(0x04034b50, 0)
    local.writeUInt16LE(recorded, 8)
    local.writeUInt32LE(body.length, 18)
    local.writeUInt32LE(data.length, 22)
    local.writeUInt16LE(nameBytes.length, 26)
    const central = Buffer.alloc(46)
    central.writeUInt32LE(0x02014b50, 0)
    central.writeUInt16LE(recorded, 10)
    central.writeUInt32LE(body.length, 20)
    central.writeUInt32LE(data.length, 24)
    central.writeUInt16LE(nameBytes.length, 28)
    central.writeUInt32LE(offset, 42)
    locals.push(local, nameBytes, body)
    centrals.push(central, nameBytes)
    offset += 30 + nameBytes.length + body.length
  })
  const directory = Buffer.concat(centrals)
  const end = Buffer.alloc(22)
  end.writeUInt32LE(0x06054b50, 0)
  end.writeUInt16LE(files.length, 8)
  end.writeUInt16LE(files.length, 10)
  end.writeUInt32LE(directory.length, 12)
  end.writeUInt32LE(offset, 16)
  return Buffer.concat([...locals, directory, end])
}

test("a wheel's member unpacks whether stored or deflated", () => {
  const library = Buffer.from("\u007fELF".repeat(1000))
  const archive = zip([
    ["onnxruntime/__init__.py", Buffer.from("# python")],
    ["onnxruntime/capi/libonnxruntime.so.1.31.1", library],
  ])
  expect(extractMember(archive, "onnxruntime/capi/libonnxruntime.so.1.31.1")).toEqual(library)
  expect(extractMember(archive, "onnxruntime/__init__.py").toString()).toBe("# python")
  expect(() => extractMember(archive, "missing")).toThrow("not in the archive")
  expect(() => extractMember(Buffer.from("not a zip at all, just some bytes"), "x")).toThrow("not a zip")
})

test("a damaged or unexpected wheel is refused, never unpacked short", () => {
  const library = Buffer.from("\u007fELF".repeat(1000))
  const files: [string, Buffer][] = [["a.py", Buffer.from("# python")], ["lib.so", library]]
  expect(() => extractMember(zip(files, 12), "lib.so"), "bzip2").toThrow("uses zip method 12")
  const good = zip(files)
  // The stored member's recorded size is wrong: it unpacks to other bytes than the directory says.
  const lying = Buffer.from(good)
  lying.writeUInt32LE(5, good.indexOf(Buffer.from([0x50, 0x4b, 0x01, 0x02])) + 24)
  expect(() => extractMember(lying, "a.py")).toThrow("unpacked to 8 bytes, not 5")
  // Cut inside the deflated member: its bytes run past the end of the archive.
  const directory = good.indexOf(Buffer.from([0x50, 0x4b, 0x01, 0x02]))
  const cut = Buffer.concat([good.subarray(0, directory - 10), good.subarray(directory)])
  expect(() => extractMember(cut, "lib.so")).toThrow()
  // A directory that points past the archive.
  const astray = Buffer.from(good)
  astray.writeUInt32LE(good.length * 2, good.length - 22 + 16)
  expect(() => extractMember(astray, "lib.so")).toThrow("a damaged zip directory")
  expect(() => extractMember(Buffer.alloc(0), "x")).toThrow("not a zip")
  expect(() => extractMember(zip([]), "x")).toThrow("not in the archive")
})

test("only the manifest's own targets are pinned", () => {
  expect(isPinned("linux-x64")).toBe(true)
  expect(isPinned("darwin-x64"), "no official wheel for Intel Macs").toBe(false)
  for (const name of ["constructor", "toString", "__proto__", ""]) expect(isPinned(name), name).toBe(false)
})

test("targets and library names match what the Runner looks for", () => {
  expect(targetFor("darwin", "arm64")).toBe("darwin-arm64")
  expect(targetFor("linux", "amd64")).toBe("linux-x64")
  expect(targetFor("win32", "x64")).toBe("windows-x64")
  expect(runtimeFile("darwin-arm64")).toBe("libonnxruntime.dylib")
  expect(runtimeFile("windows-arm64")).toBe("onnxruntime.dll")
  expect(runtimeFile("linux-x64")).toBe("libonnxruntime.so")
})

test("every pinned runtime is an official wheel with a full pin", () => {
  const { targets, version } = manifest()
  expect(Object.keys(targets).sort()).toEqual(["darwin-arm64", "linux-arm64", "linux-x64", "windows-arm64", "windows-x64"])
  for (const [target, entry] of Object.entries(targets)) {
    expect(entry.url.startsWith("https://files.pythonhosted.org/packages/"), target).toBe(true)
    expect(entry.url.includes(`onnxruntime-${version}-`), target).toBe(true)
    expect(entry.sha256).toMatch(/^[0-9a-f]{64}$/)
    expect(entry.bytes).toBeGreaterThan(1_000_000)
    expect(entry.member.startsWith("onnxruntime/capi/"), target).toBe(true)
    expect(entry.library_sha256, target).toMatch(/^[0-9a-f]{64}$/)
    expect(entry.library_bytes, target).toBeGreaterThan(1_000_000)
  }
})
