import type { MemoryRevision } from "./memoryService";

export type AssetKind = "runtime" | "model" | "tokenizer";
export type AssetSource = { kind: "supplied"; path: string } | { kind: "download"; url: string };
export interface AssetRequest { kind: AssetKind; source: AssetSource; license: string; bytes: number; sha256: string }
export interface AssetPlan { assets: AssetRequest[] }
export interface LocalAssetTarget { runner_id: string; profile_id: string; profile_revision: MemoryRevision }
export interface BotSetupTarget { runner_id: string; bot_id: string; connection_revision: MemoryRevision }
export interface LocalAssetPreview extends LocalAssetTarget { preview_token: string; preview_digest: string; expires_in_seconds: 600; total_bytes: number; assets: AssetRequest[] }
export interface InitializationTarget { database: string; database_oid: number; role: string; server_address: string | null; server_port: number | null; server_version: string; session_pid: number }
export interface SchemaReadiness { ready: boolean; extension_version: string | null; extension_schema: string | null; schema_exists: boolean; can_create_schema: boolean; can_create_tables: boolean; can_install_extension: boolean }
export interface VectorSpace { fingerprint: string; dimensions: number; distance: "cosine" | "dot" | "euclidean"; generation: number }
export type BotPreviewMethod = "memory.pgvector.initialize.preview" | "memory.lance.binding.preview" | "memory.lance.export.preview" | "memory.lance.import.preview";
interface BotPreviewBase extends BotSetupTarget { token: string; expires_at: number; profile_id: null; profile_revision: null }
export type BotSetupPreview = BotPreviewBase & (
  | { action: "memory.pgvector.initialize.preview"; details: { target: InitializationTarget; readiness: SchemaReadiness; schema: string; sql: string } }
  | { action: "memory.lance.binding.preview"; details: { directory: string; create: boolean; table: "beans_memory_v1"; namespace: string } }
  | { action: "memory.lance.export.preview" | "memory.lance.import.preview"; details: { path: string; namespace: string; space: VectorSpace; documents: number; sha256: string } }
);
export type SetupPreviewRequest =
  | { method: "memory.embeddings.local.preview"; params: LocalAssetTarget & { plan: AssetPlan } }
  | { method: "memory.pgvector.initialize.preview"; params: { bot_id: string } }
  | { method: "memory.lance.binding.preview"; params: { bot_id: string; directory: string; create: boolean } }
  | { method: "memory.lance.export.preview" | "memory.lance.import.preview"; params: { bot_id: string; path: string } };
export type BotApplyMethod = "memory.pgvector.initialize.apply" | "memory.lance.binding.apply" | "memory.lance.export.apply" | "memory.lance.import.apply";
export type SetupApplyRequest =
  | { method: "memory.embeddings.local.apply"; params: { runner_id: string; preview_token: string; preview_digest: string; confirm: true } }
  | { method: BotApplyMethod; params: { bot_id: string; token: string; confirm: true } };
export type MemorySetupRequest = SetupPreviewRequest | SetupApplyRequest | { method: "memory.embeddings.local.status"; params: { runner_id: string; profile_id: string } };
export type SetupTransport = (method: MemorySetupRequest["method"], params: MemorySetupRequest["params"]) => Promise<unknown>;
export type LocalEmbeddingStatus = "ready" | "setup_required" | "runtime_unavailable";
export type SetupResult = { installed: true; status: "ready" | "runtime_unavailable" } | { initialized: true } | { status: "ready" } | { exported: true; bytes: number } | { imported: true };

function row(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("invalid_setup_reply");
  return value as Record<string, unknown>;
}
function text(value: unknown): string {
  if (typeof value !== "string" || !value) throw new Error("invalid_setup_reply");
  return value;
}
function integer(value: unknown, min = 0, max = Number.MAX_SAFE_INTEGER): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < min || value > max) throw new Error("invalid_setup_reply");
  return value;
}
function flag(value: unknown): boolean {
  if (typeof value !== "boolean") throw new Error("invalid_setup_reply");
  return value;
}
function revision(value: unknown): MemoryRevision {
  const v = row(value);
  return { counter: integer(v.counter), device_id: text(v.device_id) };
}
function hash(value: unknown): string {
  const v = text(value);
  if (!/^[a-f0-9]{64}$/.test(v)) throw new Error("invalid_setup_reply");
  return v;
}
function assets(value: unknown): AssetRequest[] {
  if (!Array.isArray(value) || value.length !== 3) throw new Error("invalid_asset_plan");
  const found = new Set<string>();
  const result = value.map((item) => {
    const a = row(item), kind = text(a.kind) as AssetKind, source = row(a.source);
    if (!["runtime", "model", "tokenizer"].includes(kind) || found.has(kind)) throw new Error("invalid_asset_plan");
    found.add(kind);
    let boundSource: AssetSource;
    if (source.kind === "supplied") boundSource = { kind: "supplied", path: text(source.path) };
    else if (source.kind === "download") {
      const url = text(source.url);
      let parsed: URL;
      try { parsed = new URL(url); } catch { throw new Error("invalid_asset_source"); }
      if (parsed.protocol !== "https:" || parsed.username || parsed.password || parsed.hash) throw new Error("invalid_asset_source");
      boundSource = { kind: "download", url };
    } else throw new Error("invalid_asset_source");
    return { kind, source: boundSource, license: text(a.license), bytes: integer(a.bytes, 1), sha256: hash(a.sha256) };
  });
  return result;
}

/** Exact request allowlisting; every source and path remains a trusted UI setup input only. */
export function setupPreviewRequest(request: SetupPreviewRequest): SetupPreviewRequest {
  switch (request.method) {
    case "memory.embeddings.local.preview": return { method: request.method, params: { runner_id: request.params.runner_id, profile_id: request.params.profile_id, profile_revision: revision(request.params.profile_revision), plan: { assets: assets(request.params.plan.assets) } } };
    case "memory.pgvector.initialize.preview": return { method: request.method, params: { bot_id: request.params.bot_id } };
    case "memory.lance.binding.preview": return { method: request.method, params: { bot_id: request.params.bot_id, directory: request.params.directory, create: request.params.create } };
    case "memory.lance.export.preview": case "memory.lance.import.preview": return { method: request.method, params: { bot_id: request.params.bot_id, path: request.params.path } };
  }
}
export function readLocalAssetPreview(value: unknown): LocalAssetPreview {
  const p = row(value), approvedAssets = assets(p.assets), total = integer(p.total_bytes, 1);
  if (p.expires_in_seconds !== 600 || approvedAssets.reduce((sum, a) => sum + a.bytes, 0) !== total) throw new Error("invalid_setup_reply");
  return { preview_token: text(p.preview_token), preview_digest: hash(p.preview_digest), expires_in_seconds: 600, runner_id: text(p.runner_id), profile_id: text(p.profile_id), profile_revision: revision(p.profile_revision), total_bytes: total, assets: approvedAssets };
}
export function readBotSetupPreview(value: unknown, expectedAction: BotPreviewMethod): BotSetupPreview {
  const p = row(value), d = row(p.details);
  if (p.action !== expectedAction) throw new Error("approval_action_mismatch");
  if (p.profile_id !== null || p.profile_revision !== null) throw new Error("invalid_setup_reply");
  const base: BotPreviewBase = { token: text(p.token), expires_at: integer(p.expires_at), runner_id: text(p.runner_id), bot_id: text(p.bot_id), connection_revision: revision(p.connection_revision), profile_id: null, profile_revision: null };
  if (expectedAction === "memory.pgvector.initialize.preview") {
    const t = row(d.target), r = row(d.readiness);
    return { ...base, action: expectedAction, details: {
      target: { database: text(t.database), database_oid: integer(t.database_oid, 0, 4294967295), role: text(t.role), server_address: t.server_address === null ? null : text(t.server_address), server_port: t.server_port === null ? null : integer(t.server_port, -2147483648, 2147483647), server_version: text(t.server_version), session_pid: integer(t.session_pid, -2147483648, 2147483647) },
      readiness: { ready: flag(r.ready), extension_version: r.extension_version === null ? null : text(r.extension_version), extension_schema: r.extension_schema === null ? null : text(r.extension_schema), schema_exists: flag(r.schema_exists), can_create_schema: flag(r.can_create_schema), can_create_tables: flag(r.can_create_tables), can_install_extension: flag(r.can_install_extension) },
      schema: text(d.schema), sql: text(d.sql),
    } };
  }
  if (expectedAction === "memory.lance.binding.preview") {
    if (d.table !== "beans_memory_v1") throw new Error("invalid_setup_reply");
    return { ...base, action: expectedAction, details: { directory: text(d.directory), create: flag(d.create), table: "beans_memory_v1", namespace: hash(d.namespace) } };
  }
  const space = row(d.space), distance = text(space.distance);
  if (distance !== "cosine" && distance !== "dot" && distance !== "euclidean") throw new Error("invalid_setup_reply");
  return { ...base, action: expectedAction, details: { path: text(d.path), namespace: hash(d.namespace), space: { fingerprint: hash(space.fingerprint), dimensions: integer(space.dimensions, 1, 65536), distance, generation: integer(space.generation) }, documents: integer(d.documents, 0, 1000), sha256: hash(d.sha256) } };
}

const applyMethod: Record<BotPreviewMethod, BotApplyMethod> = {
  "memory.pgvector.initialize.preview": "memory.pgvector.initialize.apply",
  "memory.lance.binding.preview": "memory.lance.binding.apply",
  "memory.lance.export.preview": "memory.lance.export.apply",
  "memory.lance.import.preview": "memory.lance.import.apply",
};

export type DeepReadonly<T> = T extends readonly (infer Item)[]
  ? readonly DeepReadonly<Item>[]
  : T extends object ? { readonly [Key in keyof T]: DeepReadonly<T[Key]> } : T;

function freezePreview<T>(value: T): DeepReadonly<T> {
  if (value !== null && typeof value === "object") {
    for (const child of Object.values(value)) freezePreview(child);
    Object.freeze(value);
  }
  return value as DeepReadonly<T>;
}

export class BotSetupApproval {
  private readonly snapshot: DeepReadonly<BotSetupPreview>;
  get preview(): DeepReadonly<BotSetupPreview> { return this.snapshot; }
  private used = false;
  private readonly target: BotSetupTarget;
  private readonly token: string;
  private readonly deadline: number;
  private readonly method: BotApplyMethod;
  constructor(preview: BotSetupPreview) {
    this.snapshot = freezePreview(structuredClone(preview));
    this.target = { runner_id: this.preview.runner_id, bot_id: this.preview.bot_id, connection_revision: { ...this.preview.connection_revision } };
    this.token = this.preview.token; this.deadline = this.preview.expires_at * 1000; this.method = applyMethod[this.preview.action];
  }
  applyRequest(confirm: boolean, current: BotSetupTarget, nowMS = Date.now()): SetupApplyRequest {
    if (!confirm) throw new Error("confirmation_required");
    if (this.used) throw new Error("approval_not_found");
    if (!Number.isFinite(nowMS) || nowMS >= this.deadline) throw new Error("approval_expired");
    if (current.bot_id !== this.target.bot_id || current.runner_id !== this.target.runner_id || current.connection_revision.counter !== this.target.connection_revision.counter || current.connection_revision.device_id !== this.target.connection_revision.device_id) throw new Error("approval_stale");
    this.used = true;
    return { method: this.method, params: { bot_id: this.target.bot_id, token: this.token, confirm: true } };
  }
}
export class LocalAssetApproval {
  private readonly snapshot: DeepReadonly<LocalAssetPreview>;
  get preview(): DeepReadonly<LocalAssetPreview> { return this.snapshot; }
  private used = false;
  private readonly target: LocalAssetTarget;
  private readonly token: string;
  private readonly digest: string;
  private readonly deadline: number;
  constructor(preview: LocalAssetPreview, receivedAtMS = Date.now()) {
    this.snapshot = freezePreview(structuredClone(preview));
    this.target = { runner_id: this.preview.runner_id, profile_id: this.preview.profile_id, profile_revision: { ...this.preview.profile_revision } };
    this.token = this.preview.preview_token; this.digest = this.preview.preview_digest; this.deadline = receivedAtMS + this.preview.expires_in_seconds * 1000;
  }
  applyRequest(confirm: boolean, current: LocalAssetTarget, nowMS = Date.now()): SetupApplyRequest {
    if (!confirm) throw new Error("confirmation_required");
    if (this.used) throw new Error("approval_not_found");
    if (!Number.isFinite(nowMS) || nowMS >= this.deadline) throw new Error("approval_expired");
    if (current.runner_id !== this.target.runner_id || current.profile_id !== this.target.profile_id || current.profile_revision.counter !== this.target.profile_revision.counter || current.profile_revision.device_id !== this.target.profile_revision.device_id) throw new Error("approval_stale");
    this.used = true;
    return { method: "memory.embeddings.local.apply", params: { runner_id: this.target.runner_id, preview_token: this.token, preview_digest: this.digest, confirm: true } };
  }
}

export class MemorySetupAPI {
  constructor(private readonly transport: SetupTransport) {}
  async preview(request: SetupPreviewRequest): Promise<LocalAssetPreview | BotSetupPreview> {
    const safe = setupPreviewRequest(request), response = await this.transport(safe.method, safe.params);
    if (safe.method === "memory.embeddings.local.preview") {
      const preview = readLocalAssetPreview(response);
      if (preview.runner_id !== safe.params.runner_id || preview.profile_id !== safe.params.profile_id || preview.profile_revision.counter !== safe.params.profile_revision.counter || preview.profile_revision.device_id !== safe.params.profile_revision.device_id) throw new Error("approval_stale");
      return preview;
    }
    const preview = readBotSetupPreview(response, safe.method);
    if (preview.bot_id !== safe.params.bot_id) throw new Error("approval_stale");
    return preview;
  }
  async localStatus(runner_id: string, profile_id: string): Promise<{ status: LocalEmbeddingStatus }> {
    const reply = row(await this.transport("memory.embeddings.local.status", { runner_id, profile_id }));
    if (reply.status !== "ready" && reply.status !== "setup_required" && reply.status !== "runtime_unavailable") throw new Error("invalid_setup_reply");
    return { status: reply.status };
  }
  async applyLocal(approval: LocalAssetApproval, confirm: boolean, current: LocalAssetTarget, nowMS = Date.now()): Promise<SetupResult> {
    return this.apply(approval.applyRequest(confirm, current, nowMS));
  }
  async applyBot(approval: BotSetupApproval, confirm: boolean, current: BotSetupTarget, nowMS = Date.now()): Promise<SetupResult> {
    return this.apply(approval.applyRequest(confirm, current, nowMS));
  }
  private async apply(request: SetupApplyRequest): Promise<SetupResult> {
    const reply = row(await this.transport(request.method, request.params));
    switch (request.method) {
      case "memory.embeddings.local.apply":
        if (reply.installed !== true || (reply.status !== "ready" && reply.status !== "runtime_unavailable")) throw new Error("invalid_setup_reply");
        return { installed: true, status: reply.status };
      case "memory.pgvector.initialize.apply":
        if (reply.initialized !== true) throw new Error("invalid_setup_reply");
        return { initialized: true };
      case "memory.lance.binding.apply":
        if (reply.status !== "ready") throw new Error("invalid_setup_reply");
        return { status: "ready" };
      case "memory.lance.export.apply":
        if (reply.exported !== true) throw new Error("invalid_setup_reply");
        return { exported: true, bytes: integer(reply.bytes, 0, 8 * 1024 * 1024) };
      case "memory.lance.import.apply":
        if (reply.imported !== true) throw new Error("invalid_setup_reply");
        return { imported: true };
    }
  }
}
