// The Rust test matrix, every step CI needs in one command:
//
//   pnpm run test:rust            everything below, in order, stopping at the first failure
//   pnpm run test:rust --quick    the Runner's library tests and the ONNX Runtime tests only
//   pnpm run test:rust --live-hub the checks against the real model hub and PyPI only: every
//                                 pinned file served at its pinned size, and the measured model
//                                 downloaded, installed, and reading schedules. Needs the network
//                                 (BEANS_MODEL_HUB names a mirror); nothing else reaches it.
//
// 1. The phone core compiles without the Runner (`provider-auth` only), so nothing Runner-only
//    leaks into it; and a Runner compiles without local models (`runner` only).
// 2. The Runner crate with its default features (local models included), with the relay binary
//    the revocation test drives.
// 3. The tests that need a real ONNX Runtime (`onnx_runtime_*`, ignored by default), with the
//    pinned runtime (scripts/onnxruntime.ts) in BEANS_ONNXRUNTIME.
// 4. The phone core's own tests.
// 5. The catalog pin script's tests against a mock Hugging Face (python3; no network).

import { spawnSync } from "node:child_process"
import { join } from "node:path"
import { isPinned, placeOnnxRuntime, targetFor } from "./onnxruntime.ts"

const ROOT = join(import.meta.dirname, "..")
const quick = process.argv.includes("--quick")
const liveHub = process.argv.includes("--live-hub")

function cargo(args: string[], env: Record<string, string> = {}): void {
  console.log(`\n$ ${Object.entries(env).map(([k, v]) => `${k}=${v} `).join("")}cargo ${args.join(" ")}`)
  const run = spawnSync("cargo", args, { cwd: ROOT, stdio: "inherit", env: { ...process.env, ...env } })
  if (run.status !== 0) {
    console.error(`\nfailed: cargo ${args.join(" ")}`)
    process.exit(run.status ?? 1)
  }
}

if (liveHub) {
  cargo(["test", "-p", "beans", "--lib", "live_hub", "--", "--ignored"])
  console.log("\nthe live hub serves every pinned file")
  process.exit(0)
}

// No runtime is pinned for some hosts (Intel Macs): their ONNX Runtime tests are reported, not run.
const target = targetFor(process.platform, process.arch)
const runtime = isPinned(target) ? await placeOnnxRuntime(target, join(ROOT, "target", "onnxruntime", target)) : null
console.log(runtime ? `ONNX Runtime for ${target}: ${runtime}` : `no ONNX Runtime is pinned for ${target}: its tests will not run here`)

if (quick) {
  cargo(["test", "-p", "beans", "--lib"])
} else {
  cargo(["check", "-p", "beans", "--no-default-features", "--features", "provider-auth"])
  // A Runner built without local models still compiles and reports them unavailable.
  cargo(["check", "-p", "beans", "--no-default-features", "--features", "runner"])
  cargo(["build", "-p", "beans-relay"])
  cargo(["test", "-p", "beans", "--no-fail-fast"], { BEANS_REVOCATION_RELAY_BIN: join(ROOT, "target", "debug", "beans-relay") })
}
if (runtime) cargo(["test", "-p", "beans", "--lib", "onnx_runtime", "--", "--ignored"], { BEANS_ONNXRUNTIME: runtime })
if (!quick) cargo(["test", "-p", "beans-mobile"])
if (!quick) {
  const pins = ["-I", "-B", join("scripts", "schedule-intent-eval", "test_pin_models.py")]
  console.log(`\n$ python3 ${pins.join(" ")}`)
  const run = spawnSync("python3", pins, { cwd: ROOT, stdio: "inherit" })
  if (run.error) console.log("python3 is not installed: the pin script's tests did not run")
  else if (run.status !== 0) process.exit(run.status ?? 1)
}
console.log("\nall Rust tests passed")
