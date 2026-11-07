import { expect, test } from "vitest"
import { spawnSync } from "node:child_process"
import { readFileSync } from "node:fs"
import { fileURLToPath } from "node:url"
import { syncCatalog } from "./sync-models"

const script = fileURLToPath(new URL("./sync-models.ts", import.meta.url))
const catalog = new URL("../crates/models/catalog.json", import.meta.url)

test("the Node CLI executes offline assertions and rejects invalid arguments without writing the catalog", () => {
  const before = readFileSync(catalog)
  const checked = spawnSync(process.execPath, ["--import", "tsx", script, "--self-check"], { encoding: "utf8" })
  expect(checked.status).toBe(0)
  expect(checked.stdout).toContain("Offline model sync self-check passed; no network or catalog writes.")
  const invalid = spawnSync(process.execPath, ["--import", "tsx", script, "--report"], { encoding: "utf8" })
  expect(invalid.status).toBe(1)
  expect(invalid.stderr).toContain("Usage: pnpm run sync:models")
  expect(readFileSync(catalog)).toEqual(before)
})

test("refresh preserves extension fields at every catalog level", () => {
  const rates = { input: 1, output: 2, cache_read: 0, cache_write: 0, known: false, future: { rate: true } }
  const model = { provider: "opencode", id: "chat", name: "Chat", context_window: 100, max_output: 10,
    thinking: "effort", wire: "chat-completions", rates, future: { model: true },
    tiers: [{ input_tokens_above: 50, rates, future: { tier: true } }] }
  const input = { version: 1, updated: "2026-10-20T00:00:00Z", models: [model], review: { opencode: "chat" }, future: { catalog: true } }
  const modelsDev = Object.fromEntries(["deepseek", "anthropic", "openai", "xai", "opencode", "opencode-go"].map((p) => [p, { models: {} }]))
  modelsDev.opencode = { models: { chat: { limit: { context: 200 }, cost: { input: 0, tiers: [{ tier: { type: "context", size: 50 }, output: 3 }] } } } }
  const result = syncCatalog(JSON.stringify(input), { modelsDev, codex: [], docs: {
    opencode: "<td>chat</td><td>https://opencode.ai/inference/openai/v1/chat/completions</td>",
  } }, Date.parse("2026-10-20T01:00:00Z"))
  expect(JSON.parse(result.text)).toEqual({ ...input, updated: "2026-10-20T01:00:00Z", models: [{ ...model,
    context_window: 200, rates: { ...rates, input: 0 }, tiers: [{ ...model.tiers[0], rates: { ...rates, output: 3 } }],
  }] })
})
