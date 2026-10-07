import { expect, test } from "bun:test";
import { MemorySetupAPI, BotSetupApproval, LocalAssetApproval, readBotSetupPreview, readLocalAssetPreview, setupPreviewRequest, type AssetPlan } from "./memorySetup";

const revision = { counter: 3, device_id: "device" };
const plan: AssetPlan = { assets: ["runtime", "model", "tokenizer"].map((kind) => ({ kind: kind as "runtime" | "model" | "tokenizer", source: { kind: "supplied", path: `/fixture/${kind}` }, license: "Fixture license", bytes: 10, sha256: "a".repeat(64) })) };
const localReply = () => ({ preview_token: "opaque", preview_digest: "b".repeat(64), expires_in_seconds: 600, runner_id: "runner-a", profile_id: "local", profile_revision: revision, total_bytes: 30, assets: plan.assets });
const localTarget = () => ({ runner_id: "runner-a", profile_id: "local", profile_revision: { ...revision } });
const botReply = () => ({ token: "bot-token", expires_at: 400, action: "memory.pgvector.initialize.preview" as const, runner_id: "runner-a", bot_id: "bot-a", profile_id: null, connection_revision: revision, profile_revision: null, details: { target: { database: "memory", database_oid: 42, role: "user", server_address: null, server_port: 5432, server_version: "17", session_pid: 22 }, readiness: { ready: false, extension_version: null, extension_schema: null, schema_exists: false, can_create_schema: true, can_create_tables: true, can_install_extension: false }, schema: "beans", sql: "SELECT 'fixture preview';" } });
const botTarget = () => ({ runner_id: "runner-a", bot_id: "bot-a", connection_revision: { ...revision } });

test("local setup has exact selected Runner/profile/plan request and strips source overrides", () => {
  const request = setupPreviewRequest({ method: "memory.embeddings.local.preview", params: { ...localTarget(), plan } });
  expect(request.params).toEqual({ ...localTarget(), plan });
  expect(() => setupPreviewRequest({ method: "memory.embeddings.local.preview", params: { ...localTarget(), plan: { assets: plan.assets.slice(1) } } })).toThrow("invalid_asset_plan");
  expect(() => setupPreviewRequest({ method: "memory.embeddings.local.preview", params: { ...localTarget(), plan: { assets: plan.assets.map((a) => ({ ...a, source: { kind: "download", url: "http://fixture.invalid/model" } })) } } })).toThrow("invalid_asset_source");
});

test("local apply binds digest/Runner and consumes approval once, only after explicit consent", () => {
  const preview = readLocalAssetPreview(localReply());
  const approval = new LocalAssetApproval(preview, 0);
  expect(() => approval.applyRequest(false, localTarget(), 1)).toThrow("confirmation_required");
  expect(() => approval.applyRequest(true, { ...localTarget(), runner_id: "other" }, 1)).toThrow("approval_stale");
  expect(approval.applyRequest(true, localTarget(), 1)).toEqual({ method: "memory.embeddings.local.apply", params: { runner_id: "runner-a", preview_token: "opaque", preview_digest: "b".repeat(64), confirm: true } });
  expect(() => approval.applyRequest(true, localTarget(), 2)).toThrow("approval_not_found");
  expect(() => new LocalAssetApproval(preview, 0).applyRequest(true, localTarget(), 600000)).toThrow("approval_expired");
});

test("pgvector confirmation cannot retarget bot, connection revision, SQL or database", () => {
  const preview = readBotSetupPreview(botReply(), "memory.pgvector.initialize.preview");
  const approval = new BotSetupApproval(preview);
  expect(() => approval.applyRequest(true, { ...botTarget(), connection_revision: { counter: 4, device_id: "device" } }, 100000)).toThrow("approval_stale");
  expect(() => approval.applyRequest(false, botTarget(), 100000)).toThrow("confirmation_required");
  expect(approval.applyRequest(true, botTarget(), 100000)).toEqual({ method: "memory.pgvector.initialize.apply", params: { bot_id: "bot-a", token: "bot-token", confirm: true } });
  expect(() => readBotSetupPreview(botReply(), "memory.lance.binding.preview")).toThrow("approval_action_mismatch");
  if (preview.action !== "memory.pgvector.initialize.preview") throw new Error("wrong fixture action");
  expect(preview.details.readiness).toEqual({ ready: false, extension_version: null, extension_schema: null, schema_exists: false, can_create_schema: true, can_create_tables: true, can_install_extension: false });
});

test("Lance previews preserve truthful target/namespace/fingerprint while apply sends only token", () => {
  const body = { ...botReply(), action: "memory.lance.export.preview", details: { path: "/fixture/export.json", namespace: "c".repeat(64), space: { fingerprint: "d".repeat(64), dimensions: 3, distance: "cosine", generation: 1 }, documents: 2, sha256: "e".repeat(64) } };
  const preview = readBotSetupPreview(body, "memory.lance.export.preview");
  body.details.path = "/fixture/changed.json";
  if (preview.action !== "memory.lance.export.preview") throw new Error("wrong action");
  expect(preview.details.path).toBe("/fixture/export.json");
  expect(new BotSetupApproval(preview).applyRequest(true, botTarget(), 100000)).toEqual({ method: "memory.lance.export.apply", params: { bot_id: "bot-a", token: "bot-token", confirm: true } });
  const binding = readBotSetupPreview({ ...botReply(), action: "memory.lance.binding.preview", details: { directory: "/fixture/db", create: true, table: "beans_memory_v1", namespace: "c".repeat(64) } }, "memory.lance.binding.preview");
  expect(binding.details).toMatchObject({ create: true, table: "beans_memory_v1" });
});

test("failed apply cannot be automatically retried or represented as installed", async () => {
  const calls: string[] = [];
  const api = new MemorySetupAPI(async (method) => { calls.push(method); throw new Error("fixture_failed"); });
  const approval = new LocalAssetApproval(readLocalAssetPreview(localReply()), 0);
  await expect(api.applyLocal(approval, true, localTarget(), 1)).rejects.toThrow("fixture_failed");
  await expect(api.applyLocal(approval, true, localTarget(), 2)).rejects.toThrow("approval_not_found");
  expect(calls).toEqual(["memory.embeddings.local.apply"]);
});

test("displayed bot SQL and local asset paths are deeply immutable and match the applied token", () => {
  const bot = new BotSetupApproval(readBotSetupPreview(botReply(), "memory.pgvector.initialize.preview"));
  if (bot.preview.action !== "memory.pgvector.initialize.preview") throw new Error("wrong fixture action");
  expect(Reflect.set(bot.preview.details, "sql", "DROP TABLE changed;")).toBe(false);
  expect(Reflect.set(bot.preview.details.target, "database", "other")).toBe(false);
  expect(bot.preview.details.sql).toBe("SELECT 'fixture preview';");
  expect(bot.preview.details.target.database).toBe("memory");
  expect(bot.applyRequest(true, botTarget(), 100000).params).toEqual({ bot_id: "bot-a", token: bot.preview.token, confirm: true });
  const local = new LocalAssetApproval(readLocalAssetPreview(localReply()), 0);
  const source = local.preview.assets[0]!.source;
  expect(Reflect.set(source, "path", "/fixture/unapproved")).toBe(false);
  expect(Reflect.set(local.preview.assets[0]!, "sha256", "f".repeat(64))).toBe(false);
  expect(source).toEqual({ kind: "supplied", path: "/fixture/runtime" });
  expect(local.applyRequest(true, localTarget(), 1).params).toEqual({ runner_id: "runner-a", preview_token: local.preview.preview_token, preview_digest: local.preview.preview_digest, confirm: true });
});
