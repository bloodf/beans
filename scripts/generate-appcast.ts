// Sign Beans-<root version>.zip and write appcast.xml for beans-v<root version>.
// The parent provides BEANS_UPDATE_PRIVATE_KEY (Ed25519 PKCS8 PEM).
// No login-keychain fallback or upstream release history is used.
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs"
import { join } from "node:path"
import { RELEASES_URL, SPARKLE_TOOLS, readVersion } from "./app.ts"
import { rawReleaseSecret } from "./release-signing.ts"

/** `SPARKLE_BIN`, then the copy SwiftPM unpacked beside the framework, then PATH. */
export function sparkleTool(name: string): string | null {
  const candidates = [process.env.SPARKLE_BIN && join(process.env.SPARKLE_BIN, name), join(SPARKLE_TOOLS, name)]
  return candidates.find((path): path is string => Boolean(path) && existsSync(path as string)) ?? Bun.which(name)
}


export async function generateAppcast(updatesDir: string, downloadURLPrefix: string, requiredProtocol: number): Promise<boolean> {
  const version = readVersion()
  if (!Number.isSafeInteger(requiredProtocol) || requiredProtocol < 3) throw new Error("the release protocol must be an integer >= 3")
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
  const proc = Bun.spawn(
    [tool, "--ed-key-file", "-", "--maximum-deltas", "0", "--maximum-versions", "1",
      "--download-url-prefix", expectedPrefix, "--release-notes-url-prefix", expectedPrefix, updatesDir],
    { stdin: new Blob([secret + "\n"]), stdout: "inherit", stderr: "inherit" },
  )
  if ((await proc.exited) !== 0) return false
  const feedPath = join(updatesDir, "appcast.xml")
  const feed = readFileSync(feedPath, "utf8").replaceAll(/<beansProtocol>[^<]*<\/beansProtocol>\s*/g, "")
  if ((feed.match(/<item>/g) ?? []).length !== 1) throw new Error("expected one Beans appcast item")
  writeFileSync(feedPath, feed.replace("<item>", `<item>\n      <beansProtocol>${requiredProtocol}</beansProtocol>`))
  // Changing a custom element invalidates Sparkle's embedded feed signature. Re-sign last.
  const signed = Bun.spawn([signer, "--ed-key-file", "-", feedPath], {
    stdin: new Blob([secret + "\n"]), stdout: "inherit", stderr: "inherit",
  })
  return (await signed.exited) === 0
}

if (import.meta.main) {
  const updatesDir = process.argv[2]
  const protocol = Number(process.argv[3])
  if (!updatesDir || !Number.isSafeInteger(protocol) || protocol < 3) {
    console.error("usage: bun scripts/generate-appcast.ts <updates-dir> <protocol>")
    process.exit(1)
  }
  process.exit((await generateAppcast(updatesDir, RELEASES_URL, protocol)) ? 0 : 1)
}
