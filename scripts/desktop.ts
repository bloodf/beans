// The Windows and Linux app in desktop/, built with MyGo.
//
//   bun run desktop                       Lorca Dev with live reload: builds the CLI for this
//                                         computer and runs `mygo dev`, whose app launches it
//                                         (LORCA_CLI) with LORCA_DEV=1, as the Mac dev loop does.
//   bun run desktop:build [platforms]     The release apps: the CLI for each platform into
//                                         desktop/resources/<goos>-<goarch>/bin, then
//                                         `mygo build -platform`. Platforms are MyGo's, comma
//                                         separated (linux/amd64,windows/amd64); the default is
//                                         this computer's, or Linux and Windows on x86-64 from a
//                                         Mac. The Linux CLIs are static (musl) and, like other
//                                         computers' CLIs, build with cargo-zigbuild.
//   bun scripts/desktop.ts release <platforms> --stage-only
//                                         Signed local assets, listed in build/release-assets.json.
//   bun run release-desktop [platforms]   Upload only release assets to the existing Beans draft.

import { copyFileSync, chmodSync, mkdirSync, readdirSync } from "node:fs"
import { join, relative } from "node:path"
import { CLI_NAME, ROOT, buildCLI, color, log, readVersion, RELEASES_REPO, TAG_PREFIX } from "./app.ts"
import { extractReleaseNotes } from "./changelog.ts"
import { rawReleaseSecret } from "./release-signing.ts"

const DESKTOP = join(ROOT, "desktop")
const MYGO = join(DESKTOP, "node_modules", ".bin", process.platform === "win32" ? "mygo.exe" : "mygo")

/** The Rust target of the CLI each MyGo platform ships, static builds for Linux. */
const RUST_TARGETS: Record<string, string> = {
  "linux/amd64": "x86_64-unknown-linux-musl",
  "linux/arm64": "aarch64-unknown-linux-musl",
  "windows/amd64": "x86_64-pc-windows-gnu",
}

function hostPlatform(): string {
  const os = process.platform === "win32" ? "windows" : process.platform
  const arch = process.arch === "x64" ? "amd64" : process.arch
  return `${os}/${arch}`
}

async function run(command: string[], options: { cwd?: string; env?: Record<string, string> } = {}): Promise<number> {
  const child = Bun.spawn(command, {
    cwd: options.cwd ?? ROOT,
    env: { ...process.env, ...options.env },
    stdout: "inherit",
    stderr: "inherit",
    stdin: "inherit",
  })
  return await child.exited
}

async function dev(): Promise<number> {
  log(`${color.bold("building")} ${color.dim("the CLI (debug)")}`)
  const cli = await buildCLI("debug")
  if (!cli.ok) {
    log(color.red("the CLI did not build"))
    return 1
  }
  const binary = process.platform === "win32" ? `${cli.path}.exe` : cli.path
  log(`${color.bold("running")} ${color.dim("mygo dev")}`)
  return await run([MYGO, "dev"], { cwd: DESKTOP, env: {
    LORCA_CLI: binary,
    LORCA_DEV: "1",
    LORCA_ALLOWED_ORIGINS: "http://localhost:5178,http://127.0.0.1:5178",
  } })
}

/** The CLI for `platform`, built for its Rust target and placed where the app finds it. */
async function placeCLI(platform: string): Promise<boolean> {
  const target = RUST_TARGETS[platform]
  if (!target) {
    log(color.red(`${platform} is not a platform the desktop app ships for`))
    return false
  }
  const [goos, goarch] = platform.split("/")
  const exe = goos === "windows" ? `${CLI_NAME}.exe` : CLI_NAME
  let built: string
  if (goos === "windows" && platform === hostPlatform()) {
    // Windows builds its own CLI with its own toolchain.
    log(`${color.bold("building")} ${color.dim("the CLI (release)")}`)
    const cli = await buildCLI("release")
    if (!cli.ok) return false
    built = `${cli.path}.exe`
  } else {
    log(`${color.bold("building")} ${color.dim(`the CLI for ${target}`)}`)
    if ((await run(["cargo", "zigbuild", "--release", "--locked", "-p", CLI_NAME, "--target", target])) !== 0) return false
    built = join(ROOT, "target", target, "release", exe)
  }
  const directory = join(DESKTOP, "resources", `${goos}-${goarch}`, "bin")
  mkdirSync(directory, { recursive: true })
  const placed = join(directory, exe)
  copyFileSync(built, placed)
  if (goos !== "windows") chmodSync(placed, 0o755)
  log(`${color.green("placed")} ${color.dim(placed)}`)
  return true
}


/** Whether the release `tag` of the releases repository is a draft, published, or not there. */
function releaseState(tag: string): "draft" | "published" | "none" {
  const view = Bun.spawnSync(["gh", "release", "view", tag, "--repo", RELEASES_REPO, "--json", "isDraft", "--jq", ".isDraft"])
  if (view.exitCode !== 0) return "none"
  return view.stdout.toString().trim() === "true" ? "draft" : "published"
}

async function build(platforms: string[], options: { upload?: boolean } = {}): Promise<number> {
  const version = readVersion()
  const env = { MYGO_UPDATER_PRIVATE_KEY: await rawReleaseSecret() }
  if (options.upload) {
    if (!Bun.which("gh")) {
      log(color.red("missing the GitHub CLI, gh"))
      return 1
    }
    // MyGo can create its own draft, but the unified release orchestrator owns that lifecycle.
    if (releaseState(TAG_PREFIX + version) !== "draft") {
      log(color.red(`the parent must create the ${TAG_PREFIX}${version} draft in ${RELEASES_REPO} before uploading desktop assets`))
      return 1
    }
    if (!extractReleaseNotes(await Bun.file(join(ROOT, "CHANGELOG.md")).text(), version)) {
      log(color.red(`CHANGELOG.md has no "## [${version}]" section`))
      return 1
    }
  }
  for (const platform of platforms) {
    if (!(await placeCLI(platform))) {
      log(color.red(`the CLI for ${platform} did not build`))
      return 1
    }
  }
  log(`${color.bold("building")} ${color.dim(`the app for ${platforms.join(", ")}`)}`)
  const command = [MYGO, "build", "-platform", platforms.join(","), ...(options.upload ? ["-upload"] : [])]
  const status = await run(command, { cwd: DESKTOP, env })
  if (status === 0) {
    const assets = new Map<string, string>()
    for (const platform of platforms) {
      const target = platform.replace("/", "-")
      const directory = join(DESKTOP, "build", target)
      for (const entry of readdirSync(directory, { withFileTypes: true })) {
        if (!entry.isFile()) continue
        const name = entry.name
        if (name === `lorca-${version}-${target}.tar.gz` || name === `update-${target}.json`
          || name === `Lorca Setup ${version}.exe` || name.endsWith(".deb") || name === "install.sh") {
          assets.set(name, relative(ROOT, join(directory, name)))
        }
      }
      if (!assets.has(`lorca-${version}-${target}.tar.gz`) || !assets.has(`update-${target}.json`)) {
        throw new Error(`MyGo did not stage the signed archive and metadata for ${target}`)
      }
    }
    const manifest = join(DESKTOP, "build", "release-assets.json")
    await Bun.write(manifest, JSON.stringify({
      version, tag: TAG_PREFIX + version, assets: [...assets.values()],
    }, null, 2) + "\n")
    log(`${color.green("staged")} ${color.dim(relative(ROOT, manifest))}`)
  }
  if (status === 0 && options.upload) log(`${color.green("uploaded")} ${color.dim(`to the release ${TAG_PREFIX}${version} of ${RELEASES_REPO}`)}`)
  else if (status === 0) log(`${color.green("built")} ${color.dim(join(DESKTOP, "build"))}`)
  return status
}

const [mode, ...args] = process.argv.slice(2)
if (mode === "build" || mode === "release") {
  const stageOnly = args.includes("--stage-only")
  const unknown = args.find((arg) => arg.startsWith("--") && arg !== "--stage-only")
  const positional = args.filter((arg) => !arg.startsWith("--"))
  if (unknown || positional.length > 1) {
    throw new Error("usage: bun scripts/desktop.ts build|release [platforms] [--stage-only]")
  }
  const host = hostPlatform()
  const platforms = positional[0]?.split(",").filter((platform) => platform !== "") ?? (host.startsWith("darwin") ? ["linux/amd64", "windows/amd64"] : [host])
  process.exit(await build(platforms, { upload: mode === "release" && !stageOnly }))
}
process.exit(await dev())
