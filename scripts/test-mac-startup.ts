// Compile and run the Swift launch-concurrency tests against the real app sources.
// The executable is compiled first; a compile failure never reaches the launch step.
import { spawn, type ChildProcess } from "node:child_process"
import { mkdtemp, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"

const root = resolve(import.meta.dirname, "..")
const temporary = await mkdtemp(join(tmpdir(), "beans-launch-tests-"))

let owned: ChildProcess | null = null
/** Terminate and reap whatever this script still owns (interrupt/timeout paths included). */
async function terminateOwned() {
  const child = owned
  owned = null
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  child.kill("SIGTERM")
  await Promise.race([
    new Promise<void>((resolveExit) => child.once("exit", () => resolveExit())),
    new Promise<void>((resolveTimeout) => setTimeout(resolveTimeout, 2000)),
  ])
  if (child.exitCode === null && child.signalCode === null) {
    child.kill("SIGKILL")
    await Promise.race([
      new Promise<void>((resolveExit) => child.once("exit", () => resolveExit())),
      new Promise<void>((resolveTimeout) => setTimeout(resolveTimeout, 2000)),
    ])
  }
}

process.on("SIGINT", () => void (async () => { await terminateOwned(); process.exit(130) })())
process.on("SIGTERM", () => void (async () => { await terminateOwned(); process.exit(143) })())

/** Inherit the child's output, await its close, and treat a spawn error or a signal
 * termination as failure: only code 0 with no signal counts as success. */
async function runChild(command: string, args: string[], cwd: string): Promise<number> {
  const child = spawn(command, args, { cwd, stdio: "inherit" })
  owned = child
  const failure = new Promise<never>((_, reject) => child.once("error", reject))
  try {
    const code = await Promise.race([
      new Promise<number | null>((resolveClose) => child.once("close", resolveClose)),
      failure,
    ])
    return code ?? 1
  } finally {
    if (owned === child) owned = null
  }
}

try {
  const executable = join(temporary, "test-launch")
  if ((await runChild(
    "swiftc",
    [
      "-parse-as-library", "-swift-version", "5",
      "macos/Tests/LaunchConcurrency.swift",
      "macos/Sources/Beans/App/CLILaunchWorker.swift",
      "macos/Sources/Beans/App/StartupTrace.swift",
      "-o", executable,
    ],
    root,
  )) !== 0) {
    throw new Error("Launch test compilation failed")
  }
  if ((await runChild(executable, [temporary], root)) !== 0) {
    throw new Error("Launch tests failed")
  }
} finally {
  // Only reap and clean up after every owned child has settled.
  await terminateOwned()
  await rm(temporary, { recursive: true, force: true })
}
