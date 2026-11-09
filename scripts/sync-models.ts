import { strict as assert } from "node:assert"
// Refresh curated model metadata from public sources. New models remain report-only.
// Run `pnpm run sync:models [--report <file>] [--new <file>]` to update the local snapshot.
import { readFile, writeFile } from "node:fs/promises"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const CATALOG = join(dirname(fileURLToPath(import.meta.url)), "../crates/models/catalog.json")
const MODELS_DEV: Record<string, string> = {
  deepseek: "deepseek", anthropic: "anthropic", chatgpt: "openai", grok: "xai",
  opencode: "opencode", "opencode-go": "opencode-go",
}
const OPENCODE_DOCS: Record<string, string> = {
  opencode: "https://opencode.ai/docs/zen/", "opencode-go": "https://opencode.ai/docs/go/",
}
const CODEX_MODELS = "https://raw.githubusercontent.com/openai/codex/main/codex-rs/models-manager/models.json"
const WIRES: Record<string, string> = {
  responses: "responses", messages: "messages", "chat/completions": "chat-completions", systemone: "system-one",
}
const LEVELS = ["off", "minimal", "low", "medium", "high", "xhigh", "max"]
const RATE_FIELDS = ["input", "output", "cache_read", "cache_write"] as const
const MAX_BYTES = 1_048_576
const NEW_DAYS = 45
// Sign-in caps and provider thinking semantics remain curated, not inferred from API metadata.
const KEEP: Record<string, string[]> = {
  "grok/*": ["max_output"], "deepseek/*": ["max_output", "levels"],
  "anthropic/claude-opus-5": ["levels"], "anthropic/claude-opus-4-8": ["levels"],
  "anthropic/claude-sonnet-5-5": ["levels"], "chatgpt/*": ["levels"],
}

type Rates = Record<typeof RATE_FIELDS[number], number> & Record<string, unknown>
type Tier = { input_tokens_above: number; rates: Rates; [key: string]: unknown }
type Model = {
  provider: string; id: string; name: string; context_window: number; max_output: number
  rates: Rates; tiers?: Tier[]; thinking?: string; levels?: string[]; wire?: string
  [key: string]: unknown
}
type Catalog = { version: number; updated: string; models: Model[]; review: Record<string, string>; [key: string]: unknown }
type Sources = { modelsDev: unknown; codex?: unknown; docs: Record<string, string> }
type UpstreamModel = {
  name?: string; status?: string; release_date?: string; reasoning_options?: unknown
  limit?: { context?: number; output?: number }
  cost?: Record<string, unknown> & { tiers?: (Record<string, unknown> & { tier?: { type?: string; size?: number } })[] }
}
function record(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value)
}
function modelSource(value: unknown): Record<string, { models: Record<string, UpstreamModel> }> {
  if (!record(value)) throw new Error("Invalid models.dev source")
  for (const provider of Object.values(MODELS_DEV)) {
    const entry = value[provider]
    if (!record(entry) || !record(entry.models)) throw new Error(`Missing models.dev provider: ${provider}`)
    for (const model of Object.values(entry.models)) {
      if (!record(model)) throw new Error("Invalid upstream model")
      for (const key of ["name", "status", "release_date"]) {
        if (model[key] !== undefined && typeof model[key] !== "string") throw new Error(`Invalid upstream ${key}`)
      }
      if (model.limit !== undefined && (!record(model.limit) || [model.limit.context, model.limit.output].some((n) => n !== undefined && typeof n !== "number"))) throw new Error("Invalid upstream limits")
      if (model.cost !== undefined) {
        if (!record(model.cost)) throw new Error("Invalid upstream cost")
        if (model.cost.tiers !== undefined) {
          if (!Array.isArray(model.cost.tiers)) throw new Error("Invalid upstream tiers")
          for (const tier of model.cost.tiers) {
            if (!record(tier) || (tier.tier !== undefined && (!record(tier.tier)
              || (tier.tier.type !== undefined && typeof tier.tier.type !== "string")
              || (tier.tier.size !== undefined && typeof tier.tier.size !== "number")))) throw new Error("Invalid upstream tier")
          }
        }
      }
    }
  }
  // All fields consumed by the sync have been checked above; unrelated source fields are ignored.
  return value as Record<string, { models: Record<string, UpstreamModel> }>
}
const positive = (n: unknown): n is number => typeof n === "number" && Number.isSafeInteger(n) && n > 0
const validRate = (n: unknown): n is number => typeof n === "number" && Number.isFinite(n) && n >= 0
const completeRates = (r: unknown): r is Rates => record(r) && RATE_FIELDS.every((key) => validRate(r[key]))
const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b)

function validate(text: string, required: string[] = []): Catalog {
  if (Buffer.byteLength(text) > MAX_BYTES) throw new Error("The model catalog exceeds 1 MiB")
  const c: unknown = JSON.parse(text)
  if (!record(c) || c.version !== 1) throw new Error("The model catalog must use version 1")
  if (typeof c.updated !== "string" || !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/.test(c.updated)
    || c.updated.startsWith("0000") || !Number.isFinite(Date.parse(c.updated))
    || new Date(c.updated).toISOString().replace(".000Z", "Z") !== c.updated) throw new Error("Invalid catalog UTC time")
  if (!Array.isArray(c.models) || !c.review || typeof c.review !== "object" || Array.isArray(c.review)) throw new Error("Invalid catalog schema")
  const seen = new Set<string>()
  for (const m of c.models) {
    if (!record(m)) throw new Error("Invalid catalog model")
    const key = `${m.provider}/${m.id}`
    if (typeof m.provider !== "string" || !m.provider || typeof m.id !== "string" || !m.id
      || typeof m.name !== "string" || !m.name || seen.has(key)
      || !positive(m.context_window) || !(positive(m.max_output) || (m.wire === "system-one" && m.max_output === 0)) || !completeRates(m.rates)
      || (!(m.wire === "system-one" && m.thinking === undefined) && (typeof m.thinking !== "string" || !["adaptive", "adaptive-between-tools", "budget", "effort"].includes(m.thinking)))
      || (m.wire !== undefined && (typeof m.wire !== "string" || !Object.values(WIRES).includes(m.wire)))
      || (m.provider.startsWith("opencode") && !m.wire)
      || (m.levels !== undefined && (!Array.isArray(m.levels) || m.levels.some((l: unknown) => typeof l !== "string" || !LEVELS.includes(l))))
      || (m.tiers !== undefined && (!Array.isArray(m.tiers) || m.tiers.some((t: unknown) => !record(t) || !positive(t.input_tokens_above) || !completeRates(t.rates))))
      || [m.images, m.reasoning].some((v) => v !== undefined && typeof v !== "boolean")) throw new Error(`Invalid or unsupported catalog model: ${key}`)
    seen.add(key)
  }
  // Every model has passed the full runtime schema above; extension fields remain untouched.
  const models = c.models as Model[]
  for (const provider of new Set([...required, ...models.map((m) => m.provider)])) {
    if (!Object.hasOwn(c.review, provider)) throw new Error(`Missing review model for ${provider}`)
  }
  for (const [provider, id] of Object.entries(c.review)) {
    if (typeof id !== "string" || !models.some((m) => m.provider === provider && m.id === id)) throw new Error(`Invalid review model: ${provider}/${id}`)
  }
  // Header, rows, and review mappings have all been validated without discarding extension fields.
  return c as Catalog
}

function levelsFrom(options: unknown): string[] | undefined {
  if (!Array.isArray(options) || (options.length && options.every((o: unknown) => record(o) && o.type === "budget_tokens"))) return undefined
  const levels = new Set<string>()
  for (const option of options) {
    if (!record(option)) continue
    if (option.type === "toggle") levels.add("off")
    if (option.type === "effort" && Array.isArray(option.values)) {
      for (const value of option.values) {
        if (value === null || value === "none") levels.add("off")
        else if (typeof value === "string") levels.add(value)
      }
    }
  }
  const supported = LEVELS.filter((level) => levels.has(level))
  return supported.length || options.length === 0 ? supported : undefined
}

// Keep unknown endpoints visible. In particular, SystemOne is not Chat Completions.
function endpointTable(text: string): Map<string, string> {
  const cells = text.replace(/<script[\s\S]*?<\/script>|<style[\s\S]*?<\/style>/gi, "")
    .split(/<[^>]+>/).map((cell) => cell.trim()).filter(Boolean)
  const endpoints = new Map<string, string>()
  cells.forEach((cell, i) => {
    const endpoint = cell.match(/^https:\/\/opencode\.ai\/inference\/(?:[\w.-]+\/)*(v\d+(?:beta\d*)?)\/(.+)$/)
    if (endpoint && /^[\w.-]+$/.test(cells[i - 1] ?? "")) endpoints.set(cells[i - 1], endpoint[1] === "v1" ? endpoint[2] : `${endpoint[1]}/${endpoint[2]}`)
  })
  return endpoints
}

// Pure fixture entry point: no network, provider requests, or catalog writes.
export function syncCatalog(text: string, sources: Sources, now = Date.now()) {
  const catalog = validate(text)
  const modelsDev = modelSource(sources.modelsDev)
  const required = [...new Set(catalog.models.map((m) => m.provider))]
  const tables = Object.fromEntries(Object.entries(sources.docs).map(([p, html]) => [p, endpointTable(html)]))
  const changes: string[] = [], notes: string[] = [], fresh: string[] = []
  for (const provider of Object.keys(OPENCODE_DOCS)) {
    if (!tables[provider]?.size) {
      notes.push(`${provider}: wire verification unavailable; existing rows stay unchanged and discovery is disabled.`)
      continue
    }
    for (const [id, endpoint] of tables[provider]) {
      if (!Object.hasOwn(WIRES, endpoint)) notes.push(`${provider}/${id}: unsupported endpoint ${endpoint}; no catalog row is generated. Requires an atomic wire/parser/provider port.`)
    }
  }
  const unsupported = catalog.models.filter((m) => {
    const endpoint = tables[m.provider]?.get(m.id)
    return endpoint !== undefined && !Object.hasOwn(WIRES, endpoint)
  })
  if (unsupported.length) return {
    text, fresh: [], refused: true,
    report: `### Sync refused\n\n${notes.map((note) => `- ${note}`).join("\n")}\n\nExisting models require unsupported wires; no catalog changes are written.\n`,
  }
  for (const m of catalog.models) {
    const name = `${m.provider}/${m.id}`
    const endpoint = tables[m.provider]?.get(m.id)
    if (m.provider in OPENCODE_DOCS && !endpoint) {
      notes.push(`${name}: wire verification unavailable; existing row stays unchanged.`)
      continue
    }
    const kept = (field: string) => [`${m.provider}/${m.id}`, `${m.provider}/*`].some((key) => KEEP[key]?.includes(field))
    const set = (field: string, value: unknown) => {
      if (!same(m[field], value)) {
        changes.push(`${name}: ${field} ${JSON.stringify(m[field])} → ${JSON.stringify(value)}`)
        m[field] = value
      }
    }
    if (endpoint) set("wire", WIRES[endpoint])
    const upstream = modelsDev[MODELS_DEV[m.provider]]?.models?.[m.id]
    if (!upstream) { if (m.wire !== "system-one") notes.push(`${name} is not on models.dev.`); continue }
    if (upstream.status === "deprecated") notes.push(`${name} is deprecated on models.dev.`)
    const levels = levelsFrom(upstream.reasoning_options)
    if (m.wire !== "system-one" && levels && m.levels && !same(levels, m.levels)) {
      if (m.provider in OPENCODE_DOCS) set("levels", levels)
      else if (!kept("levels")) notes.push(`${name}: thinking levels differ; review by hand.`)
    }
    for (const [field, value] of [["context_window", upstream.limit?.context], ["max_output", upstream.limit?.output]] as const) {
      if ((positive(value) || (field === "max_output" && m.wire === "system-one" && value === 0)) && !kept(field)) set(field, value)
    }
    if (upstream.cost) {
      // Absence is unknown, not free. Preserve extension fields, including pricing completeness.
      const cost = upstream.cost
      const rates = { ...m.rates }
      for (const key of RATE_FIELDS) if (validRate(cost[key])) rates[key] = cost[key]
      set("rates", rates)
      if (!completeRates(cost)) notes.push(`${name}: incomplete or invalid upstream rates; missing components and pricing-completeness fields stay unchanged.`)
      if (upstream.cost.tiers !== undefined && !Array.isArray(upstream.cost.tiers)) throw new Error(`${name}: invalid upstream tiers`)
      const tiers = (upstream.cost.tiers ?? []).filter((t) => t.tier?.type === "context")
      if (tiers.some((t) => !completeRates(t))) notes.push(`${name}: incomplete upstream tier prices; missing components retain existing values where available.`)
      if (tiers.length) {
        if (tiers.every((t) => positive(t.tier?.size) && (completeRates(t) || m.tiers?.some((old) => old.input_tokens_above === t.tier?.size)))) {
          const unmatched = (m.tiers ?? []).filter((old) => !tiers.some((t) => t.tier?.size === old.input_tokens_above))
          if (unmatched.length) notes.push(`${name}: upstream omits existing tier thresholds; all tiers stay unchanged for manual review.`)
          else set("tiers", tiers.map((t) => {
            const old = m.tiers?.find((v) => v.input_tokens_above === t.tier?.size)
            const rates = { ...old?.rates, ...Object.fromEntries(RATE_FIELDS.filter((key) => validRate(t[key])).map((key) => [key, t[key]])) }
            return { ...old, input_tokens_above: t.tier?.size, rates }
          }))
        } else notes.push(`${name}: incomplete or invalid upstream tiers; existing tiers stay unchanged.`)
      } else if (m.tiers?.length) notes.push(`${name}: upstream no longer lists long-context tiers; review by hand.`)
    } else notes.push(`${name}: upstream pricing is missing; existing rates and pricing-completeness fields stay unchanged.`)
  }
  const listed = new Set(catalog.models.map((m) => `${m.provider}/${m.id}`))
  const since = new Date(now - NEW_DAYS * 86_400_000).toISOString().slice(0, 10)
  const family = (id: string) => id.replace(/\d+(?:\.\d+)*/g, "#")
  const version = (id: string) => (id.match(/\d+/g) ?? []).map(Number)
  const notOlder = (a: number[], b: number[]) => {
    for (let i = 0; i < Math.max(a.length, b.length); i++) if ((a[i] ?? 0) !== (b[i] ?? 0)) return (a[i] ?? 0) > (b[i] ?? 0)
    return true
  }
  const superseded = (provider: string, id: string) => catalog.models.some((m) => m.provider === provider && family(m.id) === family(id) && notOlder(version(m.id), version(id)))
  for (const [provider, source] of Object.entries(MODELS_DEV)) {
    for (const [id, e] of Object.entries(modelsDev[source]?.models ?? {})) {
      if (listed.has(`${provider}/${id}`) || e.status === "deprecated" || (e.release_date ?? "") < since || superseded(provider, id)) continue
      const endpoint = tables[provider]?.get(id)
      if (provider in OPENCODE_DOCS && (!endpoint || !Object.hasOwn(WIRES, endpoint))) continue
      if (!positive(e.limit?.output) && !(endpoint === "systemone" && e.limit?.output === 0)) continue
      const levels = levelsFrom(e.reasoning_options)
      fresh.push(`${provider}/${id} — ${e.name}, released ${e.release_date}, ${e.limit?.context} window, input ${validRate(e.cost?.input) ? `$${e.cost.input}` : "unknown"} / output ${validRate(e.cost?.output) ? `$${e.cost.output}` : "unknown"}${endpoint ? `, ${WIRES[endpoint]}` : ""}${levels ? `, levels ${JSON.stringify(levels)}` : ""}`)
    }
  }
  if (sources.codex) {
    const raw = record(sources.codex) ? sources.codex.models : sources.codex
    if (!Array.isArray(raw) || raw.some((m: unknown) => !record(m) || typeof m.slug !== "string" || typeof m.visibility !== "string")) throw new Error("Invalid Codex source")
    const visible: string[] = []
    for (const candidate of raw as unknown[]) {
      if (!record(candidate) || typeof candidate.slug !== "string") throw new Error("Invalid Codex model")
      if (candidate.visibility === "list") visible.push(candidate.slug)
    }
    const chatgpt = catalog.models.filter((m) => m.provider === "chatgpt").map((m) => m.id)
    const generation = Math.max(0, ...chatgpt.map((id) => version(id)[0] ?? 0))
    for (const slug of visible) {
      if (!chatgpt.includes(slug) && (version(slug)[0] ?? 0) >= generation && !superseded("chatgpt", slug)) fresh.push(`chatgpt/${slug} — listed in Codex's models.json`)
    }
    for (const id of chatgpt) if (!visible.includes(id)) notes.push(`chatgpt/${id} is no longer listed in Codex's models.json.`)
  } else notes.push("Codex source unavailable; ChatGPT discovery was not checked.")
  if (changes.length) catalog.updated = new Date(Math.max(now, Date.parse(catalog.updated) + 1000)).toISOString().replace(/\.\d+Z$/, "Z")
  const output = changes.length ? JSON.stringify(catalog, null, 2) + "\n" : text
  validate(output, required)
  const section = (title: string, lines: string[]) => lines.length ? `### ${title}\n\n${lines.map((l) => `- ${l}`).join("\n")}\n` : ""
  const report = [section("Changed", changes), section(`Upstream models the catalog lacks (released in the last ${NEW_DAYS} days)`, fresh), section("To check", notes)].filter(Boolean).join("\n")
  return { text: output, report, fresh, refused: false }
}

async function publicText(url: string) {
  const response = await fetch(url)
  if (!response.ok) throw new Error(`${url}: ${response.status}`)
  return response.text()
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2)
  if (args.length === 1 && args[0] === "--self-check") {
    const rates = { input: 1, output: 2, cache_read: 0, cache_write: 0, known: false }
    const row = { provider: "opencode", id: "chat", name: "Chat", context_window: 100, max_output: 10,
      thinking: "effort", wire: "chat-completions", rates, tiers: [{ input_tokens_above: 50, rates, known: false }] }
    const input = { version: 1, updated: "2026-10-20T00:00:00Z", review: { opencode: "chat" }, models: [row] }
    const modelsDev = Object.fromEntries(Object.values(MODELS_DEV).map((p) => [p, { models: {} as Record<string, UpstreamModel> }]))
    modelsDev.opencode.models.chat = { limit: { context: 200 }, cost: { input: 0, output: 0, cache_read: 0, cache_write: 0,
      tiers: [{ tier: { type: "context", size: 60 }, input: 3, output: 4, cache_read: 0, cache_write: 0 }] } }
    modelsDev.opencode.models.decision = { name: "Decision", release_date: "2026-10-20", limit: { output: 20 } }
    const html = "<td>chat</td><td>https://opencode.ai/inference/openai/v1/chat/completions</td><td>decision</td><td>https://opencode.ai/inference/systemone/v1/systemone</td>"
    const sources = { modelsDev, codex: [], docs: { opencode: html, "opencode-go": "<ul><li>No endpoint table</li></ul>" } }
    const text = JSON.stringify(input)
    const result = syncCatalog(text, sources, Date.parse("2026-10-20T01:00:00Z"))
    const output = validate(result.text)
    assert.equal(output.models[0].context_window, 200)
    assert.equal(output.models[0].rates.input, 0)
    assert.equal(output.models[0].rates.known, false)
    assert.deepEqual(output.models[0].tiers, row.tiers)
    assert.deepEqual(output.review, input.review)
    assert.equal(output.updated, "2026-10-20T01:00:00Z")
    assert.doesNotMatch(result.report, /decision: unsupported endpoint systemone/)
    assert.match(result.report, /opencode-go: wire verification unavailable/)
    assert.deepEqual(output.models.map((m) => m.id), ["chat"])
    assert.equal(result.fresh.length, 1)
    assert.match(result.fresh[0], /opencode\/decision.*system-one/)
    const decision = { ...row, id: "decision", name: "Decision", wire: "system-one", max_output: 0, thinking: undefined }
    const decisionText = JSON.stringify({ ...input, models: [row, decision] })
    const decisionResult = syncCatalog(decisionText, sources)
    assert.equal(decisionResult.refused, false)
    const decisionOutput = validate(decisionResult.text).models[1]
    assert.equal(decisionOutput.wire, "system-one")
    assert.equal(decisionOutput.thinking, undefined)
    modelsDev.opencode.models.decision.limit = { context: 64000, output: 0 }
    const zeroDiscovery = syncCatalog(text, sources, Date.parse("2026-10-20T01:00:00Z"))
    assert.equal(zeroDiscovery.fresh.length, 1)
    assert.match(zeroDiscovery.fresh[0], /opencode\/decision.*system-one/)
    const zeroOutput = syncCatalog(decisionText, sources)
    assert.equal(validate(zeroOutput.text).models[1].max_output, 0)
    assert.equal(validate(zeroOutput.text).models[1].context_window, 64000)
    assert.throws(() => validate(JSON.stringify({ ...input, models: [{ ...row, max_output: 0 }] })))
    assert.throws(() => validate(JSON.stringify({ ...input, models: [{ ...row, thinking: undefined }] })))
    assert.throws(() => validate(JSON.stringify({ ...input, models: [row, { ...decision, max_output: -1 }] })))
    assert.throws(() => validate(JSON.stringify({ ...input, models: [row, { ...decision, thinking: "unknown" }] })))
    const refused = syncCatalog(text, { ...sources, docs: { ...sources.docs, opencode: html.replace("openai/v1/chat/completions", "unknown/v1/unsupported") } })
    assert.equal(refused.refused, true)
    assert.equal(refused.text, text)
    assert.match(refused.report, /chat: unsupported endpoint unsupported/)
    assert.match(refused.report, /no catalog changes are written/)
    assert.equal(syncCatalog(text, { ...sources, docs: { opencode: "", "opencode-go": "" } }).text, text)
    modelsDev.opencode.models.chat.cost = { input: 0.25, output: 0, tiers: [{ tier: { type: "context", size: 50 }, input: 0.5 }] }
    const partial = validate(syncCatalog(text, sources).text).models[0]
    assert.deepEqual(partial.rates, { ...rates, input: 0.25, output: 0 })
    assert.deepEqual(partial.tiers, [{ ...row.tiers[0], rates: { ...rates, input: 0.5 } }])
    delete modelsDev.opencode.models.chat.cost
    const missingCost = syncCatalog(text, sources)
    assert.deepEqual(validate(missingCost.text).models[0].rates, rates)
    assert.match(missingCost.report, /upstream pricing is missing/)
    const beta = syncCatalog(text, { ...sources, docs: { ...sources.docs, opencode: `${html}<td>beta</td><td>https://opencode.ai/inference/google/v1beta/models/generateContent</td>` } })
    assert.match(beta.report, /beta: unsupported endpoint v1beta\/models\/generateContent/)
    assert.throws(() => validate(text, ["deepseek"]), /Missing review model for deepseek/)
    assert.throws(() => syncCatalog(JSON.stringify({ ...input, review: {} }), sources), /Missing review/)
    assert.throws(() => syncCatalog(JSON.stringify({ ...input, review: { opencode: "missing" } }), sources), /Invalid review model/)
    assert.throws(() => syncCatalog(JSON.stringify({ ...input, review: { opencode: 7 } }), sources), /Invalid review model/)
    assert.throws(() => syncCatalog(JSON.stringify({ ...input, version: 2 }), sources), /version 1/)
    assert.throws(() => syncCatalog(JSON.stringify({ ...input, updated: "2026-02-30T00:00:00Z" }), sources), /UTC/)
    assert.throws(() => syncCatalog(JSON.stringify({ ...input, padding: "x".repeat(MAX_BYTES) }), sources), /1 MiB/)
    const curatedLevels = JSON.stringify({ ...input, models: [{ ...row, levels: ["low", "medium"] }] })
    for (const options of [
      [{ type: "future-effort", values: ["high"] }],
      [{ type: "effort", values: ["future-level"] }],
      [{ type: "budget_tokens", values: [1024] }],
    ]) {
      modelsDev.opencode.models.chat.reasoning_options = options
      assert.deepEqual(validate(syncCatalog(curatedLevels, sources).text).models[0].levels, ["low", "medium"])
    }
    for (const [options, expected] of [
      [[], []],
      [[{ type: "effort", values: ["low", "future-level"] }], ["low"]],
      [[{ type: "toggle" }], ["off"]],
    ]) {
      modelsDev.opencode.models.chat.reasoning_options = options
      assert.deepEqual(validate(syncCatalog(curatedLevels, sources).text).models[0].levels, expected)
    }
    delete modelsDev.opencode.models.chat.reasoning_options
    console.log("Offline model sync self-check passed; no network or catalog writes.")
    process.exit(0)
  }
  const options: Record<string, string> = {}
  for (let i = 0; i < args.length; i += 2) {
    if (!["--report", "--new"].includes(args[i]) || !args[i + 1] || args[i + 1].startsWith("--")) throw new Error("Usage: pnpm run sync:models [--report <file>] [--new <file>]")
    options[args[i]] = args[i + 1]
  }
  const [text, modelsDev, codex, ...docs] = await Promise.all([
    readFile(CATALOG, "utf8"), publicText("https://models.dev/api.json").then(JSON.parse),
    publicText(CODEX_MODELS).then(JSON.parse).catch(() => undefined),
    ...Object.values(OPENCODE_DOCS).map((url) => publicText(url).catch(() => "")),
  ])
  validate(text, Object.keys(MODELS_DEV))
  const result = syncCatalog(text, { modelsDev, codex, docs: Object.fromEntries(Object.keys(OPENCODE_DOCS).map((p, i) => [p, docs[i]])) })
  if (result.text !== text) await writeFile(CATALOG, result.text)
  console.log(result.report || "The catalog matches upstream.")
  if (options["--report"]) await writeFile(options["--report"], result.report)
  if (options["--new"]) await writeFile(options["--new"], result.fresh.map((line) => `- ${line}\n`).join(""))
  if (result.refused) process.exitCode = 1
}
