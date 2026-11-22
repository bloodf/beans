// The end-to-end suites, every one that needs no device or paid service:
//
//   pnpm run test:e2e            all of them, in order, stopping at the first failure
//   pnpm run test:e2e --rust     a real relay and two `beans serve` processes: pairing, a custom
//                                provider on a local model server, and a chat through the relay
//   pnpm run test:e2e --desktop  the desktop page in Chromium (demo workspace)
//   pnpm run test:e2e --web      the website in Chromium
//
// The Rust suite builds the relay and the CLI first. The browser suites need the desktop and web
// dependencies installed and Playwright's Chromium.

import { spawnSync } from "node:child_process"
import { join } from "node:path"

const ROOT = join(import.meta.dirname, "..")
const picked = ["--rust", "--desktop", "--web"].filter((flag) => process.argv.includes(flag))
const runs = (flag: string) => picked.length === 0 || picked.includes(flag)

function run(command: string, args: string[], options: { cwd?: string; env?: Record<string, string> } = {}): void {
  console.log(`\n$ ${command} ${args.join(" ")}`)
  const child = spawnSync(command, args, { cwd: options.cwd ?? ROOT, stdio: "inherit", env: { ...process.env, ...options.env } })
  if (child.status !== 0) {
    console.error(`\nfailed: ${command} ${args.join(" ")}`)
    process.exit(child.status ?? 1)
  }
}

if (runs("--rust")) {
  run("cargo", ["build", "-p", "beans-relay", "-p", "beans"])
  run("cargo", ["test", "-p", "beans", "--test", "e2e_paired_chat", "--", "--ignored", "--test-threads=1"], {
    env: { BEANS_E2E_RELAY_BIN: join(ROOT, "target", "debug", "beans-relay") },
  })
}
if (runs("--desktop")) run("pnpm", ["--dir", "desktop", "run", "test:e2e"])
if (runs("--web")) run("pnpm", ["--dir", "web", "run", "test:e2e"])
console.log("\nall end-to-end suites passed")
