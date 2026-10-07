// Build the signed iPhone IPA for a GitHub release; never submit to a store.
//   Rust core → copied production project → pods → archive → local IPA export.
//
//   bun run release-ios                      export an ad-hoc IPA
//   bun run release-ios --local              archive only
//   bun run release-ios --export-method debugging  development-signed IPA
// Provisioning profiles and certificates must already be installed. Creating/updating
// Apple provisioning requires the explicit --allow-provisioning flag.
import { $ } from "bun"
import { existsSync } from "node:fs"
import { mkdir, readdir, rename, rm, symlink } from "node:fs/promises"
import { dirname, join, relative } from "node:path"
import { color, log, readVersion, ROOT } from "./app.ts"

function die(message: string): never {
  log(color.red(message))
  process.exit(1)
}

const args = process.argv.slice(2)
const local = args.includes("--local")
const allowProvisioning = args.includes("--allow-provisioning")
const methodIndex = args.indexOf("--export-method")
const exportMethod = methodIndex < 0 ? "release-testing" : args[methodIndex + 1]
if (exportMethod !== "release-testing" && exportMethod !== "debugging") {
  die("--export-method must be release-testing or debugging")
}
for (let index = 0; index < args.length; index++) {
  if (index === methodIndex) { index++; continue }
  if (args[index] !== "--local" && args[index] !== "--allow-provisioning") die(`unknown argument: ${args[index]}`)
}

for (const tool of ["cargo", "xcodebuild", "pod", "bunx", "plutil", "rsync"]) {
  if (!Bun.which(tool)) die(`missing required tool: ${tool}`)
}

const TEAM_ID = process.env.BEANS_APPLE_TEAM_ID ?? "GJE9R5VE87"
const BUNDLE_ID = "ai.amoena.beans"
const MOBILE = join(ROOT, "mobile")
const BUILD_DIR = join(ROOT, "dist", "ios")
// The production project is generated in a copy, so mobile/ios stays the dev loop's Beans Dev project.
// The copy stays between releases at the same path: Xcode's compilation cache keys hold absolute
// paths, and the pods installed in it are used again.
const PROJECT = join(BUILD_DIR, "mobile")
const BLOBATAR = join(ROOT, "packages", "beans-blobatar")
const COPIED_BLOBATAR = join(BUILD_DIR, "packages", "beans-blobatar")
const LINKED_BLOBATAR = join(PROJECT, "node_modules", "@beans", "blobatar")
const IOS = join(PROJECT, "ios")
// Pods and its lockfile wait here while prebuild writes a new ios/.
const KEPT = join(BUILD_DIR, "kept")
const ARCHIVE = join(BUILD_DIR, "Beans.xcarchive")
const EXPORT = join(BUILD_DIR, "export")

// Legacy ad-hoc exports require an explicitly allocated number; EAS owns store versions.
const buildNumber = process.env.BUILD_NUMBER ?? (local ? "1" : "")
if (!/^[1-9]\d*$/.test(buildNumber) || Number(buildNumber) > 2_100_000_000) {
  die("BUILD_NUMBER must be a positive integer at most 2100000000")
}

// CocoaPods dies on a non-UTF-8 locale, and the CommandLineTools SDK breaks the pod install and
// the build with "unknown architecture" from tapi. The variant variables would make a Beans Dev build.
const env: Record<string, string> = {
  ...(process.env as Record<string, string>),
  LANG: "en_US.UTF-8",
  LC_ALL: "en_US.UTF-8",
  DEVELOPER_DIR: "/Applications/Xcode.app/Contents/Developer",
  LORCA_IOS_BUILD_NUMBER: buildNumber,
}
delete env.LORCA_MOBILE_VARIANT
delete env.EAS_BUILD_PROFILE

// ---- 1. Rust core
// The xcframework is gitignored and the dev loop may hold an older one, so a release builds its own.
log(`${color.bold("core")} ${color.dim("release build for iOS")}`)
await $`bun run core ios`.cwd(MOBILE).env(env)

// ---- 2. production project
log(`${color.bold("prebuild")} ${color.dim(`Beans, ${BUNDLE_ID}, build ${buildNumber}`)}`)
// The native projects and prebuild's cache in .expo are the copy's own.
if (existsSync(join(PROJECT, "package.json"))) {
  // rsync copies only what changed, and leaves alone the xcframework that expo-modules-jsi's build
  // phase makes in its package and keys to this copy's Pods path.
  const jsi = "/node_modules/expo-modules-jsi/apple"
  await $`rsync -a --delete --exclude /ios --exclude /android --exclude /.expo --exclude ${`${jsi}/.*`} --exclude ${`${jsi}/Products`} ${`${MOBILE}/`} ${`${PROJECT}/`}`
  await mkdir(dirname(COPIED_BLOBATAR), { recursive: true })
  await $`rsync -a --delete ${`${BLOBATAR}/`} ${`${COPIED_BLOBATAR}/`}`
} else {
  // -c clones on APFS, so node_modules costs next to nothing to copy.
  await rm(PROJECT, { recursive: true, force: true })
  await mkdir(PROJECT, { recursive: true })
  for (const name of await readdir(MOBILE)) {
    if (!["ios", "android", ".expo"].includes(name)) await $`cp -cR ${join(MOBILE, name)} ${PROJECT}`
  }
  await mkdir(dirname(COPIED_BLOBATAR), { recursive: true })
  await $`cp -cR ${BLOBATAR} ${dirname(COPIED_BLOBATAR)}`
}
// app.config.ts reads its parent's release version. Keep the copied layout independent
// of the repository's Bun workspaces while preserving the exact production version.
await Bun.write(join(BUILD_DIR, "package.json"), JSON.stringify({ private: true, version: readVersion() }) + "\n")
// Bun's file: install puts absolute per-file links in node_modules. Point the copied dependency at
// the copied source instead, so Metro sees one Blobatar outside its project root and its React
// imports resolve against this mobile project's node_modules.
await rm(LINKED_BLOBATAR, { recursive: true, force: true })
await symlink(relative(dirname(LINKED_BLOBATAR), COPIED_BLOBATAR), LINKED_BLOBATAR, "dir")
// A clean prebuild writes ios/ from the Expo config. The pods installed last time go back in, so
// pod install uses them again and installs only what changed.
const KEEP = ["Pods", "Podfile.lock"]
await mkdir(KEPT, { recursive: true })
for (const name of KEEP) {
  if (!existsSync(join(IOS, name))) continue
  await rm(join(KEPT, name), { recursive: true, force: true })
  await rename(join(IOS, name), join(KEPT, name))
}
await rm(IOS, { recursive: true, force: true })
await $`bunx expo prebuild --platform ios --no-install`.cwd(PROJECT).env(env)
for (const name of KEEP) {
  if (existsSync(join(KEPT, name))) await rename(join(KEPT, name), join(IOS, name))
}
await $`pod install`.cwd(IOS).env(env)

const pbxproj = await Bun.file(join(IOS, "Beans.xcodeproj", "project.pbxproj")).text()
const bundleIds = new Set([...pbxproj.matchAll(/PRODUCT_BUNDLE_IDENTIFIER = "?([^";]+)"?;/g)].map((m) => m[1]))
if (!bundleIds.has(BUNDLE_ID)) die(`the project builds ${[...bundleIds].join(", ")}, not ${BUNDLE_ID}`)

// ---- 3. archive
// CURRENT_PROJECT_VERSION carries the build number to the notify extension, whose Info.plist reads
// it; the app's own Info.plist has it from the Expo config.
//
// An archive starts from an empty build database, so every compile runs again. Xcode's compilation
// cache (DerivedData/CompilationCache.noindex) answers the C, C++, and Objective-C ones from the
// last release: React Native's libraries drop from about seven minutes to seconds. Swift compiles
// in full, because the cache needs explicit Swift modules and React Native's prebuilt core turns
// them off.
log(`${color.bold("archiving")} ${color.dim(ARCHIVE)}`)
await rm(ARCHIVE, { recursive: true, force: true })
await rm(EXPORT, { recursive: true, force: true })
const provisioningArgs = allowProvisioning ? ["-allowProvisioningUpdates"] : []
await $`xcodebuild -workspace ${join(IOS, "Beans.xcworkspace")} -scheme Beans -configuration Release -destination generic/platform=iOS -archivePath ${ARCHIVE} ${provisioningArgs} CURRENT_PROJECT_VERSION=${buildNumber} COMPILATION_CACHE_ENABLE_CACHING=YES archive -quiet`.env(env)
if (!existsSync(ARCHIVE)) die("xcodebuild produced no archive")

const plist = join(ARCHIVE, "Products", "Applications", "Beans.app", "Info.plist")
const version = (await $`plutil -extract CFBundleShortVersionString raw ${plist}`.text()).trim()
const built = (await $`plutil -extract CFBundleVersion raw ${plist}`.text()).trim()
if (built !== buildNumber) die(`the app carries build ${built}, expected ${buildNumber}`)

if (local) {
  log(`${color.green("archived")} Beans ${version} (${buildNumber}); nothing was uploaded`)
  console.log(`  archive ${ARCHIVE}`)
  process.exit(0)
}

// ---- 4. local IPA export (no App Store or TestFlight submission)
const configuredOptions = process.env.BEANS_IOS_EXPORT_OPTIONS
const exportOptions = configuredOptions ?? join(BUILD_DIR, "ExportOptions.plist")
if (configuredOptions) {
  if (!existsSync(configuredOptions)) die("BEANS_IOS_EXPORT_OPTIONS does not exist")
  const options = JSON.parse(await $`plutil -convert json -o - ${configuredOptions}`.text())
  if (!["release-testing", "debugging", "ad-hoc", "development"].includes(options.method) || options.destination !== "export") {
    die("Export options must export an ad-hoc/development IPA locally; store upload is not allowed")
  }
} else {
  await Bun.write(exportOptions, `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>method</key><string>${exportMethod}</string>
  <key>destination</key><string>export</string>
  <key>teamID</key><string>${TEAM_ID}</string>
  <key>signingStyle</key><string>automatic</string>
  <key>manageAppVersionAndBuildNumber</key><false/>
</dict></plist>`)
}
log(`${color.bold("exporting")} ${color.dim("signed IPA for GitHub")}`)
await $`xcodebuild -exportArchive -archivePath ${ARCHIVE} -exportOptionsPlist ${exportOptions} -exportPath ${EXPORT} ${provisioningArgs}`.env(env)
const ipas = (await readdir(EXPORT)).filter((file) => file.endsWith(".ipa"))
if (ipas.length !== 1) die("xcodebuild must produce exactly one signed IPA")
const output = join(BUILD_DIR, `Beans-${version}.ipa`)
await rm(output, { force: true })
await rename(join(EXPORT, ipas[0]), output)
log(`${color.green("exported")} Beans ${version} (${buildNumber}); nothing was uploaded`)
console.log(`  IPA ${output}`)
