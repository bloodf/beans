// Checks the apps' string tables against the sources: every `L("…")` in the Mac app, every
// `t("…")` in the phone app, and every `L("…")` and `Lc("…", "…")` in the desktop app should have a
// Chinese entry, and a format key and its translation should take the same values. `pnpm run l10n` lists what is missing, unused, or mismatched;
// `--merge <fragment.json>…` adds `{ "English": "中文" }` files to the tables first.

import { spawnSync } from "node:child_process"
import { memorySwiftKeys } from "./l10n-swift.ts"
import { globSync, existsSync } from "node:fs"
import { readFile, writeFile } from "node:fs/promises"
import { join } from "node:path"

const ROOT = join(import.meta.dirname, "..")
const MAC_TABLE = join(ROOT, "macos/Resources/zh-Hans.lproj/Localizable.strings")
const PHONE_TABLE = join(ROOT, "mobile/src/i18n/zh.ts")
const DESKTOP_TABLE = join(ROOT, "desktop/src/l10n/zh.ts")

const NATIVE_TABLE = join(ROOT, "desktop/l10n/zh.go")

type Table = Map<string, string>

/// A Swift or TypeScript string literal's text, for the escapes the sources use.
function unescape(literal: string) {
  return literal.replace(/\\(u\{([0-9a-fA-F]+)\}|u([0-9a-fA-F]{4})|.)/g, (_, all, braced, plain) => {
    if (braced ?? plain) return String.fromCodePoint(parseInt(braced ?? plain, 16))
    return ({ n: "\n", t: "\t", r: "\r" } as Record<string, string>)[all] ?? all
  })
}

function escapeFor(text: string) {
  return text.replace(/\\/g, "\\\\").replace(/"/g, '\\"').replace(/\n/g, "\\n").replace(/\t/g, "\\t")
}

/// The paths under `dir` that match `pattern`, with `/` between folders on every platform:
/// scans with `\` on Windows, and the skips look for `l10n/` and `i18n/`.
async function* scan(dir: string, pattern: string) {
  for (const path of globSync(pattern, { cwd: dir })) yield path.replaceAll("\\", "/")
}

async function keysIn(dir: string, pattern: string, call: RegExp, skip: (path: string) => boolean) {
  const keys = new Map<string, string>()
  for await (const path of scan(dir, pattern)) {
    if (skip(path)) continue
    const source = await readFile(join(dir, path), "utf8")
    for (const match of source.matchAll(call)) {
      // `L("Pairing", context: "device state")` is the key `Pairing|device state`.
      if (match[1] === undefined) keys.set(unescape(match[2]), path) // a single-quoted t('…')
      else keys.set(match[2] ? `${unescape(match[1])}|${unescape(match[2])}` : unescape(match[1]), path)
    }
  }
  return keys
}

async function readMacTable(): Promise<Table> {
  const table: Table = new Map()
  if (!existsSync(MAC_TABLE)) return table
  const content = await readFile(MAC_TABLE, "utf8")
  for (const match of content.matchAll(/^"((?:[^"\\]|\\.)*)"\s*=\s*"((?:[^"\\]|\\.)*)";/gm)) {
    table.set(unescape(match[1]), unescape(match[2]))
  }
  return table
}

/// The desktop app's keys: `L("…")` or `L('…')`, and `Lc("…", "…")` as the key `…|…`.
async function desktopKeysIn(dir: string) {
  const keys = new Map<string, string>()
  for await (const path of scan(dir, "src/**/*.{ts,tsx}")) {
    if (path.includes("l10n/") || path.endsWith("mygo.ts") || path.endsWith(".test.ts")) continue
    const source = await readFile(join(dir, path), "utf8")
    for (const match of source.matchAll(/\bL\(\s*(?:"((?:[^"\\]|\\.)*)"|'((?:[^'\\]|\\.)*)')/g)) {
      keys.set(unescape(match[1] ?? match[2]), path)
    }
    for (const match of source.matchAll(/\bLc\(\s*"((?:[^"\\]|\\.)*)",\s*"((?:[^"\\]|\\.)*)"/g)) {
      keys.set(`${unescape(match[1])}|${unescape(match[2])}`, path)
    }
  }
  return keys
}

async function nativeKeysIn(dir: string) {
  const keys = new Map<string, string>()
  for await (const path of scan(dir, "**/*.go")) {
    if (path.startsWith("l10n/") || path.endsWith("_test.go")) continue
    const source = await readFile(join(dir, path), "utf8")
    for (const match of source.matchAll(/\bL\(\s*"((?:[^"\\]|\\.)*)"/g)) keys.set(unescape(match[1]), path)
    for (const match of source.matchAll(/\bLc\(\s*"((?:[^"\\]|\\.)*)",\s*"((?:[^"\\]|\\.)*)"/g)) {
      keys.set(`${unescape(match[1])}|${unescape(match[2])}`, path)
    }
  }
  return keys
}


async function readPhoneTable(path = PHONE_TABLE): Promise<Table> {
  const table: Table = new Map()
  if (!existsSync(path)) return table
  const content = await readFile(path, "utf8")
  for (const match of content.matchAll(/^\s*"((?:[^"\\]|\\.)*)":\s*"((?:[^"\\]|\\.)*)",?$/gm)) {
    table.set(unescape(match[1]), unescape(match[2]))
  }
  return table
}

function sorted(table: Table) {
  return [...table].sort(([a], [b]) => a.localeCompare(b, "en"))
}

async function writeMacTable(table: Table) {
  const lines = sorted(table).map(([key, value]) => `"${escapeFor(key)}" = "${escapeFor(value)}";`)
  await writeFile(MAC_TABLE, `/* Simplified Chinese. The key is the English text passed to L(); \`pnpm run l10n\` checks this table. */\n\n${lines.join("\n")}\n`, "utf8")
}

async function writePhoneTable(table: Table) {
  const lines = sorted(table).map(([key, value]) => `  "${escapeFor(key)}": "${escapeFor(value)}",`)
  await writeFile(PHONE_TABLE, `// Simplified Chinese. The key is the English text passed to t(); \`pnpm run l10n\` checks this table.\n\nexport const zh: Record<string, string> = {\n${lines.join("\n")}\n};\n`, "utf8")
}

async function writeDesktopTable(table: Table) {
  const lines = sorted(table).map(([key, value]) => `  "${escapeFor(key)}": "${escapeFor(value)}",`)
  await writeFile(DESKTOP_TABLE, `// Simplified Chinese. The key is the English text passed to L(); \`pnpm run l10n\` checks this table.\n\nexport const zh: Record<string, string> = {\n${lines.join("\n")}\n};\n`, "utf8")
}

async function writeNativeTable(table: Table) {
  const lines = sorted(table).map(([key, value]) => `\t"${escapeFor(key)}": "${escapeFor(value)}",`)
  await writeFile(NATIVE_TABLE, `// Simplified Chinese. Checked by pnpm run l10n.\n\npackage l10n\n\nvar zh = map[string]string{\n${lines.join("\n")}\n}\n`, "utf8")
  const formatted = spawnSync("gofmt", ["-w", NATIVE_TABLE], { stdio: "inherit" })
  if (formatted.error || formatted.status !== 0) throw new Error("gofmt failed on the native desktop table")
}

/// The values a sentence takes: `%@`/`%d` (positions ignored) or `{name}`.
function slots(text: string) {
  const printf = [...text.replace(/%%/g, "").matchAll(/%(?:\d+\$)?([@dfs])/g)].map((m) => m[1]).sort()
  const named = [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort()
  return [...printf, ...named].join(",")
}

function report(name: string, used: Map<string, string>, table: Table) {
  let problems = 0
  for (const [key, path] of used) {
    const value = table.get(key)
    if (value === undefined) {
      console.log(`${name}: no translation for ${JSON.stringify(key)} (${path})`)
      problems++
    } else if (slots(key) !== slots(value)) {
      console.log(`${name}: values differ in ${JSON.stringify(key)} → ${JSON.stringify(value)}`)
      problems++
    }
  }
  for (const key of table.keys()) {
    if (!used.has(key)) {
      console.log(`${name}: unused ${JSON.stringify(key)}`)
      problems++
    }
  }
  console.log(`${name}: ${used.size} strings, ${table.size} translated, ${problems} problems`)
  return problems
}

const macKeys = await keysIn(join(ROOT, "macos/Sources/Beans"), "**/*.swift", /\bL\(\s*"((?:[^"\\]|\\.)*)"(?:,\s*context:\s*"((?:[^"\\]|\\.)*)")?/g, () => false)
for await (const path of scan(join(ROOT, "macos/Sources/Beans"), "**/*.swift")) {
  for (const key of memorySwiftKeys(await readFile(join(ROOT, "macos/Sources/Beans", path), "utf8"), path)) macKeys.set(key, path)
}
const phoneKeys = await keysIn(join(ROOT, "mobile"), "{app,src}/**/*.{ts,tsx}", /\bt\(\s*(?:"((?:[^"\\]|\\.)*)"|'((?:[^'\\]|\\.)*)')/g, (path) => path.includes("i18n/") || path.endsWith(".test.ts"))
for (const [key, path] of await keysIn(join(ROOT, "mobile"), "{app,src}/**/*.{ts,tsx}", /\btc\(\s*"((?:[^"\\]|\\.)*)",\s*"((?:[^"\\]|\\.)*)"/g, (path) => path.includes("i18n/") || path.endsWith(".test.ts"))) phoneKeys.set(key, path)
const nativeKeys = await nativeKeysIn(join(ROOT, "desktop"))
const native = await readPhoneTable(NATIVE_TABLE)
const desktopKeys = await desktopKeysIn(join(ROOT, "desktop"))

const mac = await readMacTable()
const phone = await readPhoneTable()
const desktop = await readPhoneTable(DESKTOP_TABLE)

const mergeAt = process.argv.indexOf("--merge")
if (mergeAt !== -1) {
  for (const path of process.argv.slice(mergeAt + 1)) {
    const fragment = JSON.parse(await readFile(path, "utf8")) as Record<string, string>
    for (const [key, value] of Object.entries(fragment)) {
      for (const [used, table, name] of [[macKeys, mac, "mac"], [phoneKeys, phone, "phone"], [desktopKeys, desktop, "desktop"], [nativeKeys, native, "native"]] as const) {
        if (!used.has(key)) continue
        const existing = table.get(key)
        if (existing !== undefined && existing !== value) console.log(`${name}: kept ${JSON.stringify(existing)} over ${JSON.stringify(value)} for ${JSON.stringify(key)}`)
        else table.set(key, value)
      }
    }
  }
  await writeMacTable(mac)
  await writePhoneTable(phone)
  await writeDesktopTable(desktop)
  await writeNativeTable(native)
}

const problems = report("mac", macKeys, mac) + report("phone", phoneKeys, phone) + report("desktop", desktopKeys, desktop) + report("native", nativeKeys, native)
process.exit(problems === 0 ? 0 : 1)
