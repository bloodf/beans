// Framework-independent memory UI contract. The caller supplies the existing core transport.
// Frozen common replies/options: .omc/memory-ui-contract.md. Setup RPC wiring is core-owned.
export type MemoryBackend = "hindsight" | "open_viking" | "pgvector" | "lance_db";
export type SecretPatch = { action: "keep" } | { action: "replace"; value: string } | { action: "clear" };
export interface RecallBudget { timeout_ms: number; max_bytes: number; max_results: number; max_context_chars: number }
export interface MemoryPreferences {
  connection_id: string | null;
  auto_recall: boolean;
  capture_conversation: boolean;
  capture_group_text: boolean;
  unattended_capture: boolean;
  max_capture_deliveries_per_turn: number;
  recall_budget: RecallBudget;
}
export interface MemoryConnectionEdit {
  id: string;
  backend: MemoryBackend;
  name: string;
  endpoint?: string | null;
  secret: SecretPatch;
  embedding_profile?: string | null;
  allow_insecure_http?: boolean;
  options?: MemoryConnectionOptions;
}
export type MemoryConnectionOptions =
  | { backend: "hindsight" }
  | { backend: "pgvector"; schema: string; role?: string | null }
  | { backend: "lance_db"; region: string | null }
  | { backend: "open_viking"; bindings: Record<string, { mode: "user_key" | "trusted_gateway"; account_id: string; user_id: string; secret: SecretPatch } | null>; replace_all?: boolean; confirm_replace_all?: boolean };
export interface MemoryRevision { counter: number; device_id: string }
export interface MemoryPreferencesView extends MemoryPreferences { consent_revision: MemoryRevision; deletion_epoch: number }
export interface MemoryConnectionView { id: string; revision: MemoryRevision; backend: MemoryBackend; name: string; has_secret: boolean; embedding_profile: string | null; availability: "supported" | "blocked"; reason: string | null }
export interface MemoryEmbeddingView { id: string; revision: MemoryRevision; model: string; model_revision: string; dimensions: number; has_secret: boolean }
export interface MemoryConnectionsView { schema_version: 1; connections: MemoryConnectionView[]; embeddings: MemoryEmbeddingView[]; bots: { bot_id: string; preferences: MemoryPreferencesView }[] }
export interface MemoryHealth { status: "ready" | "degraded" | "setup_required"; capabilities: MemoryCapabilities; deletion_pending: boolean; reason?: string | null }
export interface MemoryOperation { id: string; document_id: string; state: MemoryOperationState; operation_id: string | null; error_code: string | null }
export interface MemoryOperations { operations: MemoryOperation[]; deletion: { pending: boolean; operation_id: string | null; deletion_epoch: number } | null }
export type MemoryAdvancedFeature = "bank_profile" | "bank_config" | "directives" | "mental_models" | "mental_model_history" | "observations" | "memory_edit" | "memory_invalidate" | "memory_restore" | "documents" | "sessions" | "resources" | "tasks";
export type MemoryBasicAction = "retain" | "recall" | "inspect" | "delete_document" | "clear" | "operation_status" | "cancel_operation" | "reflect";
export type MemoryCapabilities = Partial<Record<MemoryBasicAction | "idempotent_retain" | "write_fence", boolean>> & { advanced?: readonly MemoryAdvancedFeature[]; advanced_actions?: Partial<Record<MemoryAdvancedFeature, readonly string[]>> };
export const MEMORY_ADVANCED_ACTIONS: Record<MemoryAdvancedFeature, readonly string[]> = {
  bank_profile: ["get", "update", "reset"], bank_config: ["get", "update", "reset"],
  directives: ["list", "get", "create", "update", "delete"], mental_models: ["list", "get", "create", "update", "refresh", "history", "delete"],
  mental_model_history: ["list", "get"], observations: ["list", "scopes", "clear", "clear_derived"],
  memory_edit: ["get", "history", "update", "edit"], memory_invalidate: ["get", "history", "update"], memory_restore: ["get", "history", "update"],
  documents: ["list", "get", "chunks"], sessions: ["list", "get", "create", "add_message", "commit", "delete"],
  resources: ["list", "read", "add", "delete"], tasks: ["list", "get", "cancel", "delete_record"],
};
export type MemoryOperationState = "queued" | "submitted" | "processing" | "completed" | "failed" | "delivery_unknown";
export type MemoryRequest =
  | { method: "memory.connections.list"; params: Record<string, never> }
  | { method: "memory.connections.set"; params: MemoryConnectionEdit }
  | { method: "memory.connections.disconnect"; params: { id: string } }
  | { method: "memory.preferences.get" | "memory.service.health" | "memory.operations.list"; params: { bot_id: string } }
  | { method: "memory.preferences.set"; params: MemoryPreferences & { bot_id: string } };
export type MemoryTransport = (method: MemoryRequest["method"], params: MemoryRequest["params"]) => Promise<unknown>;

export function secretPatch(action: SecretPatch["action"], value = ""): SecretPatch {
  if (action !== "replace") return { action };
  const bytes = new TextEncoder().encode(value).length;
  if (bytes === 0 || bytes > 8192) throw new Error("invalid_secret");
  return { action, value };
}

export function connectionRequest(edit: MemoryConnectionEdit): Extract<MemoryRequest, { method: "memory.connections.set" }> {
  // No spread: form data cannot introduce namespace/bank/tenant or arbitrary backend JSON.
  const params: MemoryConnectionEdit = {
    id: edit.id, backend: edit.backend, name: edit.name,
    secret: edit.secret.action === "replace" ? secretPatch("replace", edit.secret.value) : secretPatch(edit.secret.action),
  };
  if (edit.endpoint !== undefined) params.endpoint = edit.endpoint;
  if (edit.embedding_profile !== undefined) params.embedding_profile = edit.embedding_profile;
  if (edit.allow_insecure_http !== undefined) params.allow_insecure_http = edit.allow_insecure_http;
  if (edit.options !== undefined) {
    if (edit.options.backend !== edit.backend) throw new Error("backend_options_mismatch");
    switch (edit.options.backend) {
      case "hindsight": params.options = { backend: "hindsight" }; break;
      case "pgvector":
        params.options = { backend: "pgvector", schema: edit.options.schema };
        if (edit.options.role !== undefined) params.options.role = edit.options.role;
        break;
      case "lance_db": params.options = { backend: "lance_db", region: edit.options.region }; break;
      case "open_viking": {
        if (edit.options.replace_all === true && edit.options.confirm_replace_all !== true) throw new Error("confirmation_required");
        const bindings: Extract<MemoryConnectionOptions, { backend: "open_viking" }>["bindings"] = Object.create(null);
        for (const [botID, binding] of Object.entries(edit.options.bindings)) {
          if (binding === null) { bindings[botID] = null; continue; }
          bindings[botID] = {
            mode: binding.mode, account_id: binding.account_id, user_id: binding.user_id,
            secret: binding.secret.action === "replace" ? secretPatch("replace", binding.secret.value) : secretPatch(binding.secret.action),
          };
        }
        params.options = { backend: "open_viking", bindings };
        if (edit.options.replace_all !== undefined) params.options.replace_all = edit.options.replace_all;
        if (edit.options.confirm_replace_all !== undefined) params.options.confirm_replace_all = edit.options.confirm_replace_all;
        break;
      }
      default: throw new Error("invalid_backend_options");
    }
  }
  return { method: "memory.connections.set", params };
}

export function supportsMemoryAction(capabilities: MemoryCapabilities | undefined, action: MemoryBasicAction | MemoryAdvancedFeature): boolean {
  if (!capabilities) return false;
  switch (action) {
    case "retain": case "recall": case "inspect": case "delete_document": case "clear":
    case "operation_status": case "cancel_operation": case "reflect":
      return capabilities[action] === true;
    default: return capabilities.advanced?.includes(action) === true;
  }
}

export function supportsMemoryAdvancedAction(capabilities: MemoryCapabilities | undefined, feature: MemoryAdvancedFeature, action: string): boolean {
  if (feature === "documents" && action === "delete") return false;
  return capabilities?.advanced?.includes(feature) === true && MEMORY_ADVANCED_ACTIONS[feature].includes(action) && capabilities.advanced_actions?.[feature]?.includes(action) === true;
}

export function memoryVectorEditBody(documentID: string, text: string): { document_id: string; text: string } {
  if (!documentID || documentID.length > 256 || /[\u0000-\u001f\u007f]/.test(documentID) || !text || new TextEncoder().encode(text).length > 32768) throw new Error("invalid_memory_edit");
  return { document_id: documentID, text };
}

const initialPreferences = (): MemoryPreferences => ({
  connection_id: null, auto_recall: false, capture_conversation: false, capture_group_text: false,
  unattended_capture: false, max_capture_deliveries_per_turn: 1,
  recall_budget: { timeout_ms: 2000, max_bytes: 16384, max_results: 8, max_context_chars: 4000 },
});

function validateBudget(budget: RecallBudget) {
  for (const [value, min, max] of [
    [budget.timeout_ms, 1, 5000], [budget.max_bytes, 1, 32768],
    [budget.max_results, 1, 20], [budget.max_context_chars, 100, 8000],
  ]) {
    if (value === undefined || min === undefined || max === undefined || !Number.isSafeInteger(value) || value < min || value > max) throw new Error("invalid_budget");
  }
}

/** Ephemeral consent belongs to this bot/connection and this exact draft, never to the account. */
export class MemoryPreferencesDraft {
  readonly botID: string;
  private connection: string | null;
  private readonly original: MemoryPreferences;
  private originalConsentValid = true;
  private plaintextApproval: string | undefined;
  private groupApproval: string | undefined;
  autoRecall: boolean;
  captureConversation: boolean;
  captureGroupText: boolean;
  unattendedCapture: boolean;
  maxCaptureDeliveriesPerTurn: number;
  recallBudget: RecallBudget;

  constructor(botID: string, saved: MemoryPreferences = initialPreferences()) {
    this.botID = botID;
    this.original = { ...saved, recall_budget: { ...saved.recall_budget } };
    this.connection = saved.connection_id;
    this.autoRecall = saved.auto_recall;
    this.captureConversation = saved.capture_conversation;
    this.captureGroupText = saved.capture_group_text;
    this.unattendedCapture = saved.unattended_capture;
    this.maxCaptureDeliveriesPerTurn = saved.max_capture_deliveries_per_turn;
    this.recallBudget = { ...saved.recall_budget };
  }

  get connectionID() { return this.connection; }
  set connectionID(value: string | null) {
    if (value === this.connection) return;
    this.connection = value;
    this.plaintextApproval = this.groupApproval = undefined;
    this.originalConsentValid = false;
  }

  private consentTarget() {
    return JSON.stringify([this.botID, this.connection, this.autoRecall, this.captureConversation, this.captureGroupText, this.maxCaptureDeliveriesPerTurn]);
  }
  approveRemotePlaintext() { this.plaintextApproval = this.consentTarget(); }
  approveGroupCapture() { this.groupApproval = this.consentTarget(); }

  request(): Extract<MemoryRequest, { method: "memory.preferences.set" }> {
    if (this.unattendedCapture) throw new Error("enforced_spending_policy_required");
    if (!Number.isInteger(this.maxCaptureDeliveriesPerTurn) || this.maxCaptureDeliveriesPerTurn < 0 || this.maxCaptureDeliveriesPerTurn > 4) throw new Error("invalid_capture_cap");
    validateBudget(this.recallBudget);
    if (this.captureGroupText && !this.captureConversation) throw new Error("group_capture_requires_conversation");
    if ((this.autoRecall || this.captureConversation) && !this.connection) throw new Error("connection_required");
    const retainsOriginalConsent = this.originalConsentValid && this.connection === this.original.connection_id;
    const needsPlaintext = (this.autoRecall && !(retainsOriginalConsent && this.original.auto_recall)) ||
      (this.captureConversation && !(retainsOriginalConsent && this.original.capture_conversation && this.maxCaptureDeliveriesPerTurn <= this.original.max_capture_deliveries_per_turn));
    if (needsPlaintext && this.plaintextApproval !== this.consentTarget()) throw new Error("plaintext_consent_required");
    if (this.captureGroupText && !(retainsOriginalConsent && this.original.capture_group_text) && this.groupApproval !== this.consentTarget()) throw new Error("group_consent_required");
    return {
      method: "memory.preferences.set",
      params: {
        bot_id: this.botID, connection_id: this.connection, auto_recall: this.autoRecall,
        capture_conversation: this.captureConversation, capture_group_text: this.captureGroupText,
        unattended_capture: false, max_capture_deliveries_per_turn: this.maxCaptureDeliveriesPerTurn,
        recall_budget: { ...this.recallBudget },
      },
    };
  }
}

/** Validated, allowlisted masked replies never retain unexpected secret fields. */
export class MemoryServiceAPI {
  constructor(private readonly transport: MemoryTransport) {}
  private send(request: MemoryRequest) { return this.transport(request.method, request.params); }
  async listConnections() { return readMemoryConnections(await this.send({ method: "memory.connections.list", params: {} })); }
  setConnection(edit: MemoryConnectionEdit) { return this.send(connectionRequest(edit)); }
  disconnectConnection(id: string) { return this.send({ method: "memory.connections.disconnect", params: { id } }); }
  async getPreferences(botID: string) { return readMemoryPreferences(await this.send({ method: "memory.preferences.get", params: { bot_id: botID } })); }
  savePreferences(draft: MemoryPreferencesDraft) { return Promise.resolve().then(() => this.send(draft.request())); }
  async health(botID: string) { return readMemoryHealth(await this.send({ method: "memory.service.health", params: { bot_id: botID } })); }
  async operations(botID: string) { return readMemoryOperations(await this.send({ method: "memory.operations.list", params: { bot_id: botID } })); }
}

const advancedFeatures: readonly MemoryAdvancedFeature[] = ["bank_profile", "bank_config", "directives", "mental_models", "mental_model_history", "observations", "memory_edit", "memory_invalidate", "memory_restore", "documents", "sessions", "resources", "tasks"];
const operationStates: readonly MemoryOperationState[] = ["queued", "submitted", "processing", "completed", "failed", "delivery_unknown"];
const backendKinds: readonly MemoryBackend[] = ["hindsight", "open_viking", "pgvector", "lance_db"];

function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("invalid_memory_reply");
  return value as Record<string, unknown>;
}
function text(value: unknown): string {
  if (typeof value !== "string") throw new Error("invalid_memory_reply");
  return value;
}
function nullableText(value: unknown): string | null {
  if (value === null) return null;
  return text(value);
}
function flag(value: unknown): boolean {
  if (typeof value !== "boolean") throw new Error("invalid_memory_reply");
  return value;
}
function count(value: unknown): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) throw new Error("invalid_memory_reply");
  return value;
}
function array(value: unknown): unknown[] {
  if (!Array.isArray(value)) throw new Error("invalid_memory_reply");
  return value;
}
function revision(value: unknown): MemoryRevision {
  const row = object(value);
  return { counter: count(row.counter), device_id: text(row.device_id) };
}

export function readMemoryPreferences(value: unknown): MemoryPreferencesView {
  const row = object(value), budget = object(row.recall_budget);
  const recall_budget = { timeout_ms: count(budget.timeout_ms), max_bytes: count(budget.max_bytes), max_results: count(budget.max_results), max_context_chars: count(budget.max_context_chars) };
  validateBudget(recall_budget);
  const cap = count(row.max_capture_deliveries_per_turn);
  if (cap > 4) throw new Error("invalid_memory_reply");
  return {
    connection_id: nullableText(row.connection_id), auto_recall: flag(row.auto_recall), capture_conversation: flag(row.capture_conversation),
    capture_group_text: flag(row.capture_group_text), unattended_capture: flag(row.unattended_capture),
    max_capture_deliveries_per_turn: cap, recall_budget,
    consent_revision: revision(row.consent_revision), deletion_epoch: count(row.deletion_epoch),
  };
}

export function readMemoryConnections(value: unknown): MemoryConnectionsView {
  const row = object(value);
  if (row.schema_version !== 1) throw new Error("invalid_memory_reply");
  return {
    schema_version: 1,
    connections: array(row.connections).map((value) => {
      const c = object(value), backend = text(c.backend) as MemoryBackend;
      if (!backendKinds.includes(backend)) throw new Error("invalid_memory_reply");
      const availability = text(c.availability);
      if (availability !== "supported" && availability !== "blocked") throw new Error("invalid_memory_reply");
      return { id: text(c.id), revision: revision(c.revision), backend, name: text(c.name), has_secret: flag(c.has_secret), embedding_profile: nullableText(c.embedding_profile), availability, reason: nullableText(c.reason) };
    }),
    embeddings: array(row.embeddings).map((value) => {
      const e = object(value);
      return { id: text(e.id), revision: revision(e.revision), model: text(e.model), model_revision: text(e.model_revision), dimensions: count(e.dimensions), has_secret: flag(e.has_secret) };
    }),
    bots: array(row.bots).map((value) => {
      const b = object(value);
      return { bot_id: text(b.bot_id), preferences: readMemoryPreferences(b.preferences) };
    }),
  };
}

export function readMemoryHealth(value: unknown): MemoryHealth {
  const row = object(value), status = text(row.status);
  if (status !== "ready" && status !== "degraded" && status !== "setup_required") throw new Error("invalid_memory_reply");
  const supplied = object(row.capabilities), capabilities: MemoryCapabilities = {};
  for (const name of ["retain", "recall", "inspect", "delete_document", "clear", "operation_status", "cancel_operation", "reflect", "idempotent_retain", "write_fence"] as const) {
    if (supplied[name] !== undefined) capabilities[name] = flag(supplied[name]);
  }
  if (supplied.advanced_actions !== undefined) {
    const negotiated = object(supplied.advanced_actions);
    capabilities.advanced_actions = {};
    for (const feature of advancedFeatures) {
      if (negotiated[feature] !== undefined) capabilities.advanced_actions[feature] = array(negotiated[feature]).map(text).filter((action) => !(feature === "documents" && action === "delete"));
    }
  }
  capabilities.advanced = supplied.advanced === undefined ? [] : array(supplied.advanced).map(text).filter((name): name is MemoryAdvancedFeature => advancedFeatures.includes(name as MemoryAdvancedFeature));
  return { status, capabilities, deletion_pending: flag(row.deletion_pending), ...(row.reason === undefined ? {} : { reason: nullableText(row.reason) }) };
}

export function readMemoryOperations(value: unknown): MemoryOperations {
  const row = object(value), deletion = row.deletion === null ? null : object(row.deletion);
  return {
    operations: array(row.operations).map((value) => {
      const o = object(value), state = text(o.state) as MemoryOperationState;
      if (!operationStates.includes(state)) throw new Error("invalid_memory_reply");
      return { id: text(o.id), document_id: text(o.document_id), state, operation_id: nullableText(o.operation_id), error_code: nullableText(o.error_code) };
    }),
    deletion: deletion === null ? null : { pending: flag(deletion.pending), operation_id: nullableText(deletion.operation_id), deletion_epoch: count(deletion.deletion_epoch) },
  };
}

/** Authoritative status may advance independently of a user's unsaved preference draft. */
export class MemoryServiceViewState {
  readonly draft: MemoryPreferencesDraft;
  private latest: MemoryPreferencesView;
  private minimumDeletionEpoch: number;
  private needsDeletionRefresh = false;
  constructor(botID: string, preferences: MemoryPreferencesView) {
    this.latest = structuredClone(preferences);
    this.minimumDeletionEpoch = preferences.deletion_epoch;
    this.draft = new MemoryPreferencesDraft(botID, preferences);
  }
  get preferences() { return this.latest; }
  get deletionLocked() { return this.needsDeletionRefresh; }
  beginDeletion(): number {
    if (this.needsDeletionRefresh) throw new Error("preferences_refresh_required");
    this.needsDeletionRefresh = true;
    return this.latest.deletion_epoch;
  }
  recordDeletionEpoch(epoch: number) {
    if (!Number.isSafeInteger(epoch) || epoch < this.latest.deletion_epoch) throw new Error("invalid_memory_reply");
    this.minimumDeletionEpoch = Math.max(this.minimumDeletionEpoch, epoch);
  }
  refreshPreferences(preferences: MemoryPreferencesView) {
    if (preferences.deletion_epoch < this.minimumDeletionEpoch) throw new Error("stale_preferences");
    this.latest = structuredClone(preferences);
    this.minimumDeletionEpoch = preferences.deletion_epoch;
    this.needsDeletionRefresh = false;
  }
}
