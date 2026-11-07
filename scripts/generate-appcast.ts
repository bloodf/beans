// Sign Beans-<root version>.zip and write appcast.xml for beans-v<root version>.
// The parent provides BEANS_UPDATE_PRIVATE_KEY (Ed25519 PKCS8 PEM).
// No login-keychain fallback or upstream release history is used.
import { spawn } from "node:child_process"
import { existsSync, readdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs"
import { join, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { RELEASES_URL, SPARKLE_TOOLS, readVersion } from "./app.ts"
import { findExecutable } from "./executable.ts"
import { rawReleaseSecret } from "./release-signing.ts"

/** `SPARKLE_BIN`, then the copy SwiftPM unpacked beside the framework, then PATH. */
export function sparkleTool(name: string): string | null {
  const candidates = [process.env.SPARKLE_BIN && join(process.env.SPARKLE_BIN, name), join(SPARKLE_TOOLS, name)]
  return candidates.find((path): path is string => Boolean(path) && existsSync(path as string)) ?? findExecutable(name)
}

/** Run a Sparkle tool with the seed on stdin: secrets never appear in argv. */
function runTool(command: string[], secret: string): Promise<number | null> {
  return new Promise((settle, reject) => {
    const child = spawn(command[0]!, command.slice(1), { stdio: ["pipe", "inherit", "inherit"] })
    // A tool that stops reading closes the pipe: that is the tool's answer, not a crash.
    child.stdin?.on("error", () => {})
    child.once("error", reject)
    child.once("close", (code) => settle(code))
    child.stdin?.end(`${secret}\n`)
  })
}

export async function generateAppcast(updatesDir: string, downloadURLPrefix: string, requiredProtocol: number): Promise<boolean> {
  const version = readVersion()
  if (!Number.isSafeInteger(requiredProtocol) || requiredProtocol < 5) throw new Error("the release protocol must be an integer >= 5")
  const expectedPrefix = `https://github.com/bloodf/beans/releases/download/beans-v${version}/`
  if (downloadURLPrefix !== expectedPrefix) throw new Error("Sparkle downloads must use the current Beans GitHub release")
  const archives = readdirSync(updatesDir).filter((name) => /\.(zip|dmg|delta)$/.test(name))
  if (archives.length !== 1 || archives[0] !== `Beans-${version}.zip`) {
    throw new Error(`the appcast directory must hold only Beans-${version}.zip as its update archive`)
  }
  // MyGo uses seed32 + public32, but this Sparkle version takes the seed32 alone.
  const secret = Buffer.from(await rawReleaseSecret(), "base64").subarray(0, 32).toString("base64")
  const tool = sparkleTool("generate_appcast")
  const signer = sparkleTool("sign_update")
  if (!tool || !signer) {
    console.error("generate_appcast and sign_update are required: resolve Sparkle in macos/, or set SPARKLE_BIN")
    return false
  }
  // Sparkle accepts a base64 seed on stdin. Secrets never appear in argv or the keychain.
  const code = await runTool(
    [tool, "--ed-key-file", "-", "--maximum-deltas", "0", "--maximum-versions", "1",
      "--download-url-prefix", expectedPrefix, "--release-notes-url-prefix", expectedPrefix, updatesDir],
    secret,
  )
  if (code !== 0) return false
  const feedPath = join(updatesDir, "appcast.xml")
  const feed = readFileSync(feedPath, "utf8").replaceAll(/<beansProtocol>[^<]*<\/beansProtocol>\s*/g, "")
  if ((feed.match(/<item>/g) ?? []).length !== 1) throw new Error("expected one Beans appcast item")
  writeFileSync(feedPath, feed.replace("<item>", `<item>\n      <beansProtocol>${requiredProtocol}</beansProtocol>`))
  // Changing a custom element invalidates Sparkle's embedded feed signature. Re-sign last.
  return (await runTool([signer, "--ed-key-file", "-", feedPath], secret)) === 0
}

/** Dispatch only when this file is the process entry; importing it must stay inert. */
const isMain = (() => {
  const meta = import.meta as ImportMeta & { main?: boolean }
  if (meta.main === true) return true
  const entry = process.argv[1]
  if (!entry) return false
  const canonical = (path: string) => { try { return realpathSync(path) } catch { return resolve(path) } }
  try {
    return canonical(fileURLToPath(meta.url)) === canonical(resolve(entry))
  } catch {
    return false
  }
})()

if (isMain) {
  const updatesDir = process.argv[2]
  const protocol = Number(process.argv[3])
  if (!updatesDir || !Number.isSafeInteger(protocol) || protocol < 5) {
    console.error("usage: pnpm exec tsx scripts/generate-appcast.ts <updates-dir> <protocol>")
    process.exit(1)
  }
  process.exit((await generateAppcast(updatesDir, RELEASES_URL, protocol)) ? 0 : 1)
}
