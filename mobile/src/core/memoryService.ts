// Phone-only port of the frozen masked memory contract; no vector/runtime dependency.
// Frozen common replies/options: .omc/memory-ui-contract.md. Setup RPC wiring is core-owned.
export type MemoryBackend = "hindsight" | "open_viking" | "pgvector" | "lance_db";
export type SecretPatch = {
    action: "keep";
} | {
    action: "replace";
    value: string;
} | {
    action: "clear";
};
export interface RecallBudget {
    timeout_ms: number;
    max_bytes: number;
    max_results: number;
    max_context_chars: number;
}
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
export type MemoryConnectionOptions = {
    backend: "hindsight";
} | {
    backend: "pgvector";
    schema: string;
} | {
    backend: "lance_db";
    region: string | null;
} | {
    backend: "open_viking";
    bindings: Record<string, {
        mode: "user_key" | "trusted_gateway";
        account_id: string;
        user_id: string;
        secret: SecretPatch;
    } | null>;
};
export interface MemoryRevision {
    counter: number;
    device_id: string;
}
export interface MemoryPreferencesView extends MemoryPreferences {
    consent_revision: MemoryRevision;
    deletion_epoch: number;
}
export interface MemoryConnectionView {
    id: string;
    revision: MemoryRevision;
    backend: MemoryBackend;
    name: string;
    has_secret: boolean;
    embedding_profile: string | null;
    availability: "supported" | "blocked";
    reason: string | null;
}
export interface MemoryEmbeddingView {
    id: string;
    revision: MemoryRevision;
    model: string;
    model_revision: string;
    dimensions: number;
    has_secret: boolean;
}
export interface MemoryConnectionsView {
    schema_version: 1;
    connections: MemoryConnectionView[];
    embeddings: MemoryEmbeddingView[];
    bots: {
        bot_id: string;
        preferences: MemoryPreferencesView;
    }[];
}
export interface MemoryHealth {
    status: "ready" | "degraded" | "setup_required";
    capabilities: MemoryCapabilities;
    deletion_pending: boolean;
    reason?: string;
}
export interface MemoryOperation {
    id: string;
    document_id: string;
    state: MemoryOperationState;
    operation_id: string | null;
    error_code: string | null;
}
export interface MemoryOperations {
    operations: MemoryOperation[];
    deletion: {
        pending: boolean;
        operation_id: string | null;
        deletion_epoch: number;
    } | null;
}
export type MemoryAdvancedFeature = "bank_profile" | "bank_config" | "directives" | "mental_models" | "mental_model_history" | "observations" | "memory_edit" | "memory_invalidate" | "memory_restore" | "documents" | "sessions" | "resources" | "tasks";
export type MemoryBasicAction = "retain" | "recall" | "inspect" | "delete_document" | "clear" | "operation_status" | "cancel_operation" | "reflect";
export type MemoryCapabilities = Partial<Record<MemoryBasicAction | "idempotent_retain" | "write_fence", boolean>> & {
    advanced?: readonly MemoryAdvancedFeature[];
    advanced_actions?: Partial<Record<MemoryAdvancedFeature, readonly string[]>>;
};
export type MemoryOperationState = "queued" | "submitted" | "processing" | "completed" | "failed" | "delivery_unknown";
export type MemoryRequest = {
    method: "memory.connections.list";
    params: Record<string, never>;
} | {
    method: "memory.connections.set";
    params: MemoryConnectionEdit;
} | {
    method: "memory.connections.disconnect";
    params: {
        id: string;
    };
} | {
    method: "memory.preferences.get" | "memory.service.health" | "memory.operations.list";
    params: {
        bot_id: string;
    };
} | {
    method: "memory.preferences.set";
    params: MemoryPreferences & {
        bot_id: string;
    };
};
export type MemoryTransport = (method: string, params: unknown) => Promise<unknown>;
export const LANCE_CLOUD_UNAVAILABLE_REASON = "LanceDB Cloud transport is unavailable until an upstream safe transport hook exists (lancedb_cloud_transport_unavailable). Credentials cannot enable it. Runner-local LanceDB is separate; no paid or local fallback.";
export function secretPatch(action: SecretPatch["action"], value = ""): SecretPatch {
    if (action !== "replace")
        return { action };
    const bytes = new TextEncoder().encode(value).length;
    if (bytes === 0 || bytes > 8192)
        throw new Error("invalid_secret");
    return { action, value };
}
export function connectionActivationReason(connection: MemoryConnectionView | undefined): string | undefined {
    if (!connection)
        return "Choose and save a connection first.";
    return connection.availability === "supported" ? undefined : connection.reason ?? "Connection is blocked.";
}
export function connectionRequest(edit: MemoryConnectionEdit): Extract<MemoryRequest, {
    method: "memory.connections.set";
}> {
    if (edit.backend === "lance_db" && ((edit.endpoint !== undefined && edit.endpoint !== null)
        || edit.secret.action === "replace"
        || (edit.options?.backend === "lance_db" && edit.options.region !== null)))
        throw new Error("lancedb_cloud_transport_unavailable");
    // No spread: form data cannot introduce namespace/bank/tenant or arbitrary backend JSON.
    const params: MemoryConnectionEdit = {
        id: edit.id, backend: edit.backend, name: edit.name,
        secret: edit.secret.action === "replace" ? secretPatch("replace", edit.secret.value) : secretPatch(edit.secret.action),
    };
    if (edit.endpoint !== undefined)
        params.endpoint = edit.endpoint;
    if (edit.embedding_profile !== undefined)
        params.embedding_profile = edit.embedding_profile;
    if (edit.allow_insecure_http !== undefined)
        params.allow_insecure_http = edit.allow_insecure_http;
    if (edit.options !== undefined) {
        if (edit.options.backend !== edit.backend)
            throw new Error("backend_options_mismatch");
        switch (edit.options.backend) {
            case "hindsight":
                params.options = { backend: "hindsight" };
                break;
            case "pgvector":
                params.options = { backend: "pgvector", schema: edit.options.schema };
                break;
            case "lance_db":
                params.options = { backend: "lance_db", region: edit.options.region };
                break;
            case "open_viking": {
                const bindings: Extract<MemoryConnectionOptions, {
                    backend: "open_viking";
                }>["bindings"] = Object.create(null);
                for (const [botID, binding] of Object.entries(edit.options.bindings)) {
                    if (binding === null) {
                        bindings[botID] = null;
                        continue;
                    }
                    bindings[botID] = {
                        mode: binding.mode, account_id: binding.account_id, user_id: binding.user_id,
                        secret: binding.secret.action === "replace" ? secretPatch("replace", binding.secret.value) : secretPatch(binding.secret.action),
                    };
                }
                params.options = { backend: "open_viking", bindings };
                break;
            }
            default: throw new Error("invalid_backend_options");
        }
    }
    return { method: "memory.connections.set", params };
}
export function supportsMemoryAction(capabilities: MemoryCapabilities | undefined, action: MemoryBasicAction | MemoryAdvancedFeature, verb?: string): boolean {
    if (!capabilities)
        return false;
    switch (action) {
        case "retain":
        case "recall":
        case "inspect":
        case "delete_document":
        case "clear":
        case "operation_status":
        case "cancel_operation":
        case "reflect":
            return capabilities[action] === true;
        default: return capabilities.advanced?.includes(action) === true
            && capabilities.advanced_actions?.[action]?.some(value =>
                !(action === "documents" && value === "delete") && (verb === undefined || value === verb)) === true;
    }
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
        if (value === undefined || min === undefined || max === undefined || !Number.isSafeInteger(value) || value < min || value > max)
            throw new Error("invalid_budget");
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
        if (value === this.connection)
            return;
        this.connection = value;
        this.plaintextApproval = this.groupApproval = undefined;
        this.originalConsentValid = false;
    }
    private consentTarget() {
        return JSON.stringify([this.botID, this.connection, this.autoRecall, this.captureConversation, this.captureGroupText, this.maxCaptureDeliveriesPerTurn]);
    }
    approveRemotePlaintext() { this.plaintextApproval = this.consentTarget(); }
    approveGroupCapture() { this.groupApproval = this.consentTarget(); }
    request(): Extract<MemoryRequest, {
        method: "memory.preferences.set";
    }> {
        if (this.unattendedCapture)
            throw new Error("enforced_spending_policy_required");
        if (!Number.isInteger(this.maxCaptureDeliveriesPerTurn) || this.maxCaptureDeliveriesPerTurn < 0 || this.maxCaptureDeliveriesPerTurn > 4)
            throw new Error("invalid_capture_cap");
        validateBudget(this.recallBudget);
        if (this.captureGroupText && !this.captureConversation)
            throw new Error("group_capture_requires_conversation");
        if ((this.autoRecall || this.captureConversation) && !this.connection)
            throw new Error("connection_required");
        const retainsOriginalConsent = this.originalConsentValid && this.connection === this.original.connection_id;
        const needsPlaintext = (this.autoRecall && !(retainsOriginalConsent && this.original.auto_recall)) ||
            (this.captureConversation && !(retainsOriginalConsent && this.original.capture_conversation && this.maxCaptureDeliveriesPerTurn <= this.original.max_capture_deliveries_per_turn));
        if (needsPlaintext && this.plaintextApproval !== this.consentTarget())
            throw new Error("plaintext_consent_required");
        if (this.captureGroupText && !(retainsOriginalConsent && this.original.capture_group_text) && this.groupApproval !== this.consentTarget())
            throw new Error("group_consent_required");
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
class MemoryConfigAPI {
    constructor(protected readonly transport: MemoryTransport) { }
    private send(request: MemoryRequest) { return this.transport(request.method, request.params); }
    async listConnections() { return readMemoryConnections(await this.send({ method: "memory.connections.list", params: {} })); }
    setConnection(edit: MemoryConnectionEdit) { return this.send(connectionRequest(edit)); }
    disconnectConnection(id: string) { return this.send({ method: "memory.connections.disconnect", params: { id } }); }
    async getPreferences(botID: string) { return readMemoryPreferences(await this.send({ method: "memory.preferences.get", params: { bot_id: botID } })); }
    async savePreferences(draft: MemoryPreferencesDraft, config: MemoryConnectionsView) {
        if (draft.connectionID !== null) {
            const reason = connectionActivationReason(config.connections.find(c => c.id === draft.connectionID));
            if (reason)
                throw new Error(reason);
        }
        return this.send(draft.request());
    }
    async health(botID: string) { return readMemoryHealth(await this.send({ method: "memory.service.health", params: { bot_id: botID } })); }
    async operations(botID: string) { return readMemoryOperations(await this.send({ method: "memory.operations.list", params: { bot_id: botID } })); }
}
const advancedFeatures: readonly MemoryAdvancedFeature[] = ["bank_profile", "bank_config", "directives", "mental_models", "mental_model_history", "observations", "memory_edit", "memory_invalidate", "memory_restore", "documents", "sessions", "resources", "tasks"];
const operationStates: readonly MemoryOperationState[] = ["queued", "submitted", "processing", "completed", "failed", "delivery_unknown"];
const backendKinds: readonly MemoryBackend[] = ["hindsight", "open_viking", "pgvector", "lance_db"];
function object(value: unknown): Record<string, unknown> {
    if (!value || typeof value !== "object" || Array.isArray(value))
        throw new Error("invalid_memory_reply");
    return value as Record<string, unknown>;
}
function text(value: unknown): string {
    if (typeof value !== "string")
        throw new Error("invalid_memory_reply");
    return value;
}
function nullableText(value: unknown): string | null {
    if (value === null)
        return null;
    return text(value);
}
function flag(value: unknown): boolean {
    if (typeof value !== "boolean")
        throw new Error("invalid_memory_reply");
    return value;
}
function count(value: unknown): number {
    if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0)
        throw new Error("invalid_memory_reply");
    return value;
}
function array(value: unknown): unknown[] {
    if (!Array.isArray(value))
        throw new Error("invalid_memory_reply");
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
    if (cap > 4)
        throw new Error("invalid_memory_reply");
    return {
        connection_id: nullableText(row.connection_id), auto_recall: flag(row.auto_recall), capture_conversation: flag(row.capture_conversation),
        capture_group_text: flag(row.capture_group_text), unattended_capture: flag(row.unattended_capture),
        max_capture_deliveries_per_turn: cap, recall_budget,
        consent_revision: revision(row.consent_revision), deletion_epoch: count(row.deletion_epoch),
    };
}
export function readMemoryConnections(value: unknown): MemoryConnectionsView {
    const row = object(value);
    if (row.schema_version !== 1)
        throw new Error("invalid_memory_reply");
    return {
        schema_version: 1,
        connections: array(row.connections).map((value) => {
            const c = object(value), backend = text(c.backend) as MemoryBackend;
            if (!backendKinds.includes(backend))
                throw new Error("invalid_memory_reply");
            const availability = text(c.availability);
            if (availability !== "supported" && availability !== "blocked")
                throw new Error("invalid_memory_reply");
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
    if (status !== "ready" && status !== "degraded" && status !== "setup_required")
        throw new Error("invalid_memory_reply");
    const supplied = object(row.capabilities), capabilities: MemoryCapabilities = {};
    for (const name of ["retain", "recall", "inspect", "delete_document", "clear", "operation_status", "cancel_operation", "reflect", "idempotent_retain", "write_fence"] as const) {
        if (supplied[name] !== undefined)
            capabilities[name] = flag(supplied[name]);
    }
    capabilities.advanced = supplied.advanced === undefined ? [] : array(supplied.advanced).map(text).filter((name): name is MemoryAdvancedFeature => advancedFeatures.includes(name as MemoryAdvancedFeature));
    const actions = supplied.advanced_actions === undefined ? {} : object(supplied.advanced_actions);
    capabilities.advanced_actions = {};
    for (const feature of capabilities.advanced) {
        capabilities.advanced_actions[feature] = actions[feature] === undefined ? [] : array(actions[feature]).map(text)
            .filter(action => !(feature === "documents" && action === "delete"));
    }
    return { status, capabilities, deletion_pending: flag(row.deletion_pending), ...(row.reason === undefined ? {} : { reason: text(row.reason) }) };
}
export function readMemoryOperations(value: unknown): MemoryOperations {
    const row = object(value), deletion = row.deletion === null ? null : object(row.deletion);
    return {
        operations: array(row.operations).map((value) => {
            const o = object(value), state = text(o.state) as MemoryOperationState;
            if (!operationStates.includes(state))
                throw new Error("invalid_memory_reply");
            return { id: text(o.id), document_id: text(o.document_id), state, operation_id: nullableText(o.operation_id), error_code: nullableText(o.error_code) };
        }),
        deletion: deletion === null ? null : { pending: flag(deletion.pending), operation_id: nullableText(deletion.operation_id), deletion_epoch: count(deletion.deletion_epoch) },
    };
}
export interface LocalModelConfig {
    model_sha256: string;
    tokenizer_sha256: string;
    max_tokens: number;
    pooling: "mean" | "cls" | "pooled";
    tensors: {
        input_ids: string;
        attention_mask: string;
        output: string;
        token_type_ids: string | null;
    };
    add_special_tokens: boolean;
    pad_id: number;
    pad_type_id: number;
    pad_token: string;
}
export interface EmbeddingProfileEdit {
    model: string;
    revision: string;
    dimensions: number;
    normalization: "none" | "l2";
    distance: "cosine" | "dot" | "euclidean";
    document_prefix: string;
    query_prefix: string;
    endpoint: string | null;
    mode: "api" | "local_cpu";
    local: LocalModelConfig | null;
}
export interface AssetRequest {
    kind: "runtime" | "model" | "tokenizer";
    source: {
        kind: "supplied";
        path: string;
    } | {
        kind: "download";
        url: string;
    };
    license: string;
    bytes: number;
    sha256: string;
}
export interface AssetPlan {
    assets: AssetRequest[];
}
export interface LocalPreview {
    preview_token: string;
    preview_digest: string;
    expires_in_seconds: number;
    runner_id: string;
    profile_id: string;
    profile_revision: MemoryRevision;
    total_bytes: number;
    assets: AssetRequest[];
}
export type BotSetupAction = "pgvector.initialize" | "lance.binding" | "lance.export" | "lance.import";
export interface InitializationTarget {
    database: string;
    database_oid: number;
    role: string;
    server_address: string | null;
    server_port: number | null;
    server_version: string;
    session_pid: number;
}
export interface VectorSpace {
    fingerprint: string;
    dimensions: number;
    distance: "cosine" | "dot" | "euclidean";
    generation: number;
}
export type SetupDetails = {
    target: InitializationTarget;
    schema: string;
    sql: string;
} | {
    directory: string;
    create: boolean;
    table: "beans_memory_v1";
    namespace: string;
} | {
    path: string;
    namespace: string;
    space: VectorSpace;
    documents: number;
    sha256: string;
};
export interface BotPreview {
    token: string;
    expires_at: number;
    action: string;
    runner_id: string;
    bot_id: string;
    connection_revision: MemoryRevision;
    details: SetupDetails;
}
export interface MemoryResponse {
    evidence: {
        id: string;
        text: string;
        document_id: string | null;
    }[];
    document: {
        id: string;
        text: string;
        request_id: string;
        content_hash: string;
        sources: { chat_id: string; message_id: string; speaker: "user" | "assistant" }[];
    } | null;
    operation: {
        id: string;
        state: MemoryOperationState;
    } | null;
    data: unknown;
}
export interface LocalMemoryView {
    text: string;
    hash: string;
    lines: number;
    bytes: number;
    truncated: boolean;
    max_lines: number;
    max_bytes: number;
}
function requireAction(caps: MemoryCapabilities, action: MemoryBasicAction | MemoryAdvancedFeature, approved = true, verb?: string) {
    if (!supportsMemoryAction(caps, action, verb))
        throw new Error("unsupported_operation");
    if (!approved)
        throw new Error("plaintext_consent_required");
}
function boundedText(value: string, max: number) {
    if (!value.trim() || new TextEncoder().encode(value).length > max)
        throw new Error("invalid_memory_text");
}
function boundedQuery(value: string) {
    let characters = 0;
    for (const _ of value) characters++;
    if (!value.trim() || characters > 4096) throw new Error("invalid_memory_query");
}
function readOperation(value: unknown): MemoryOperation {
    return readMemoryOperations({ operations: [value], deletion: null }).operations[0]!;
}
function readResponse(value: unknown): MemoryResponse {
    const row = object(value);
    const document = row.document === null ? null : object(row.document);
    const operation = row.operation === null ? null : object(row.operation);
    if (operation && !operationStates.includes(operation.state as MemoryOperationState))
        throw new Error("invalid_memory_reply");
    return {
        evidence: array(row.evidence).map(value => {
            const e = object(value);
            return { id: text(e.id), text: text(e.text), document_id: nullableText(e.document_id) };
        }),
        document: document ? {
            id: text(document.id), text: text(document.text), request_id: text(document.request_id),
            content_hash: text(document.content_hash), sources: array(document.sources).map(value => {
                const source = object(value);
                if (source.speaker !== "user" && source.speaker !== "assistant") throw new Error("invalid_memory_reply");
                return { chat_id: text(source.chat_id), message_id: text(source.message_id), speaker: source.speaker };
            }),
        } : null,
        operation: operation ? { id: text(operation.id), state: operation.state as MemoryOperationState } : null,
        data: row.data,
    };
}
export function validateAssetPlan(plan: AssetPlan): AssetPlan {
    if (plan.assets.length !== 3 || new Set(plan.assets.map(a => a.kind)).size !== 3)
        throw new Error("invalid_asset_plan");
    return { assets: plan.assets.map(a => {
            if (!["runtime", "model", "tokenizer"].includes(a.kind) || !/^[a-f0-9]{64}$/.test(a.sha256) ||
                !Number.isSafeInteger(a.bytes) || a.bytes <= 0 || !a.license.trim())
                throw new Error("invalid_asset_plan");
            const source = a.source.kind === "supplied"
                ? { kind: "supplied" as const, path: a.source.path }
                : { kind: "download" as const, url: a.source.url };
            if (source.kind === "supplied" && !/^(\/|[A-Za-z]:[\\/])/.test(source.path))
                throw new Error("runner_absolute_path_required");
            if (source.kind === "download") {
                const url = new URL(source.url);
                if (url.protocol !== "https:" || url.username || url.password || url.hash)
                    throw new Error("invalid_asset_url");
            }
            return { kind: a.kind, source, license: a.license, bytes: a.bytes, sha256: a.sha256 };
        }) };
}
export function embeddingRequest(id: string, p: EmbeddingProfileEdit, secret: SecretPatch) {
    if (!p.model.trim() || !p.revision.trim() || !Number.isSafeInteger(p.dimensions) || p.dimensions < 1 || p.dimensions > 65536)
        throw new Error("invalid_embedding_profile");
    if (p.mode === "local_cpu" && !p.local)
        throw new Error("local_model_contract_required");
    if (!["none", "l2"].includes(p.normalization) || !["cosine", "dot", "euclidean"].includes(p.distance))
        throw new Error("invalid_embedding_profile");
    if (p.mode === "api") {
        if (!p.endpoint)
            throw new Error("embedding_endpoint_required");
        const url = new URL(p.endpoint);
        if (!["http:", "https:"].includes(url.protocol) || url.username || url.password || url.search || url.hash || !url.pathname.endsWith("/embeddings"))
            throw new Error("invalid_embedding_endpoint");
    }
    else if (p.mode === "local_cpu" && p.local) {
        const local = p.local;
        if (!/^[a-f0-9]{64}$/.test(local.model_sha256) || !/^[a-f0-9]{64}$/.test(local.tokenizer_sha256) ||
            !Number.isSafeInteger(local.max_tokens) || local.max_tokens <= 0 ||
            !["mean", "cls", "pooled"].includes(local.pooling) ||
            !local.tensors.input_ids || !local.tensors.attention_mask || !local.tensors.output ||
            !Number.isSafeInteger(local.pad_id) || local.pad_id < 0 || local.pad_id > 4294967295 ||
            !Number.isSafeInteger(local.pad_type_id) || local.pad_type_id < 0 || local.pad_type_id > 4294967295)
            throw new Error("invalid_local_model_contract");
    }
    else
        throw new Error("invalid_embedding_mode");
    return { id, profile: {
            model: p.model, revision: p.revision, dimensions: p.dimensions, normalization: p.normalization,
            distance: p.distance, document_prefix: p.document_prefix, query_prefix: p.query_prefix,
            endpoint: p.mode === "api" ? p.endpoint : null, mode: p.mode,
            local: p.mode === "local_cpu" && p.local ? {
                model_sha256: p.local.model_sha256, tokenizer_sha256: p.local.tokenizer_sha256,
                max_tokens: p.local.max_tokens, pooling: p.local.pooling,
                tensors: { input_ids: p.local.tensors.input_ids, attention_mask: p.local.tensors.attention_mask,
                    output: p.local.tensors.output, token_type_ids: p.local.tensors.token_type_ids }, add_special_tokens: p.local.add_special_tokens,
                pad_id: p.local.pad_id, pad_type_id: p.local.pad_type_id, pad_token: p.local.pad_token,
            } : null,
        }, secret: secret.action === "replace" ? secretPatch("replace", secret.value) : secretPatch(secret.action) };
}
/** All service/setup calls use the existing native request transport; core routes to Runners. */
export class MemoryServiceAPI extends MemoryConfigAPI {
    private readonly approvals = new Map<object, {
        expires: number;
        snapshot: string;
    }>();
    async recall(bot_id: string, query: string, caps: MemoryCapabilities, approved: boolean) {
        requireAction(caps, "recall", approved);
        boundedQuery(query);
        return readResponse(await this.transport("memory.service.recall", { bot_id, query }));
    }
    async reflect(bot_id: string, query: string, caps: MemoryCapabilities, approved: boolean) {
        requireAction(caps, "reflect", approved);
        boundedQuery(query);
        return readResponse(await this.transport("memory.service.reflect", { bot_id, query }));
    }
    async retain(bot_id: string, value: string, request_id: string, caps: MemoryCapabilities, approved: boolean) {
        requireAction(caps, "retain", approved);
        boundedText(value, 32768);
        return readOperation(await this.transport("memory.service.retain", { bot_id, text: value, request_id }));
    }
    async inspect(bot_id: string, document_id: string, caps: MemoryCapabilities) {
        requireAction(caps, "inspect");
        return readResponse(await this.transport("memory.service.inspect", { bot_id, document_id }));
    }
    async operation(bot_id: string, id: string, action: "retry" | "status" | "cancel", caps: MemoryCapabilities, approved: boolean) {
        requireAction(caps, action === "retry" ? "retain" : action === "status" ? "operation_status" : "cancel_operation", approved);
        return readOperation(await this.transport(`memory.operations.${action}`, { bot_id, id }));
    }
    async delete(bot_id: string, document_id: string | undefined, connection_revision: MemoryRevision, deletion_epoch: number, caps: MemoryCapabilities, confirm: boolean) {
        requireAction(caps, document_id ? "delete_document" : "clear");
        if (!confirm)
            throw new Error("confirmation_required");
        const row = object(await this.transport("memory.service.delete", { bot_id, ...(document_id ? { document_id } : {}), connection_revision, deletion_epoch, confirm: true }));
        return { pending: flag(row.pending), deletion_epoch: count(row.deletion_epoch), operation_id: row.operation_id == null ? null : text(row.operation_id) };
    }
    async advanced(bot_id: string, feature: MemoryAdvancedFeature, action: string, body: Record<string, unknown>, caps: MemoryCapabilities, approved: boolean) {
        requireAction(caps, feature, approved, action);
        if (Object.keys(body).some(key => ["bank", "namespace", "tenant", "account", "uri", "table", "connection", "key", "api_key"].includes(key)))
            throw new Error("scope_selector_forbidden");
        return readResponse(await this.transport("memory.service.advanced", { bot_id, feature, action, body }));
    }
    setEmbedding(id: string, profile: EmbeddingProfileEdit, secret: SecretPatch) {
        return this.transport("memory.embeddings.set", embeddingRequest(id, profile, secret));
    }
    removeEmbedding(id: string) { return this.transport("memory.embeddings.remove", { id }); }
    async botPreview(action: BotSetupAction, bot_id: string, input?: {
        directory: string;
        create: boolean;
    } | {
        path: string;
    }) {
        const row = object(await this.transport(`memory.${action}.preview`, { bot_id, ...input }));
        if (row.action !== `memory.${action}.preview` || row.bot_id !== bot_id)
            throw new Error("approval_action_mismatch");
        const d = object(row.details);
        let details: SetupDetails;
        if (action === "pgvector.initialize") {
            const t = object(d.target);
            details = { schema: text(d.schema), sql: text(d.sql), target: {
                    database: text(t.database), database_oid: count(t.database_oid), role: text(t.role),
                    server_address: nullableText(t.server_address), server_port: t.server_port === null ? null : count(t.server_port),
                    server_version: text(t.server_version), session_pid: count(t.session_pid),
                } };
        }
        else if (action === "lance.binding") {
            if (d.table !== "beans_memory_v1")
                throw new Error("invalid_memory_reply");
            details = { directory: text(d.directory), create: flag(d.create), table: d.table, namespace: text(d.namespace) };
        }
        else {
            const s = object(d.space);
            if (!["cosine", "dot", "euclidean"].includes(text(s.distance)))
                throw new Error("invalid_memory_reply");
            details = { path: text(d.path), namespace: text(d.namespace), documents: count(d.documents), sha256: text(d.sha256),
                space: { fingerprint: text(s.fingerprint), dimensions: count(s.dimensions), distance: s.distance as VectorSpace["distance"], generation: count(s.generation) } };
        }
        if (typeof row.expires_at !== "number" || !Number.isFinite(row.expires_at))
            throw new Error("invalid_memory_reply");
        const preview: BotPreview = { token: text(row.token), action: text(row.action), expires_at: row.expires_at, runner_id: text(row.runner_id), bot_id, connection_revision: revision(row.connection_revision), details };
        this.approvals.set(preview, { expires: preview.expires_at * 1000, snapshot: JSON.stringify(preview) });
        return preview;
    }
    async applyBotPreview(preview: BotPreview, confirm: boolean) {
        this.consume(preview, confirm);
        return this.transport(preview.action.replace(/preview$/, "apply"), { bot_id: preview.bot_id, token: preview.token, confirm: true });
    }
    async localPreview(runner_id: string, profile_id: string, profile_revision: MemoryRevision, plan: AssetPlan) {
        const row = object(await this.transport("memory.embeddings.local.preview", { runner_id, profile_id, profile_revision, plan: validateAssetPlan(plan) }));
        if (row.runner_id !== runner_id || row.profile_id !== profile_id || JSON.stringify(revision(row.profile_revision)) !== JSON.stringify(profile_revision))
            throw new Error("approval_stale");
        const assets = validateAssetPlan({ assets: array(row.assets).map(value => {
                const a = object(value), s = object(a.source);
                if (s.kind !== "supplied" && s.kind !== "download")
                    throw new Error("invalid_memory_reply");
                return { kind: text(a.kind) as AssetRequest["kind"], source: s.kind === "supplied" ? { kind: s.kind, path: text(s.path) } : { kind: s.kind, url: text(s.url) }, license: text(a.license), bytes: count(a.bytes), sha256: text(a.sha256) };
            }) }).assets;
        const preview: LocalPreview = { preview_token: text(row.preview_token), preview_digest: text(row.preview_digest), expires_in_seconds: count(row.expires_in_seconds), runner_id, profile_id, profile_revision: revision(row.profile_revision), total_bytes: count(row.total_bytes), assets };
        if (preview.total_bytes !== assets.reduce((sum, a) => sum + a.bytes, 0))
            throw new Error("invalid_memory_reply");
        this.approvals.set(preview, { expires: Date.now() + preview.expires_in_seconds * 1000, snapshot: JSON.stringify(preview) });
        return preview;
    }
    async applyLocalPreview(preview: LocalPreview, confirm: boolean) {
        this.consume(preview, confirm);
        return this.transport("memory.embeddings.local.apply", { runner_id: preview.runner_id, preview_token: preview.preview_token, preview_digest: preview.preview_digest, confirm: true });
    }
    async localStatus(runner_id: string, profile_id: string) {
        const row = object(await this.transport("memory.embeddings.local.status", { runner_id, profile_id }));
        if (!["ready", "setup_required", "runtime_unavailable"].includes(text(row.status)))
            throw new Error("invalid_memory_reply");
        return text(row.status);
    }
    async readLocal(bot_id: string): Promise<LocalMemoryView> {
        const row = object(await this.transport("memory.read", { bot_id })), index = object(row.index);
        return { text: text(index.text), hash: text(index.hash), lines: count(index.lines), bytes: count(index.bytes),
            truncated: flag(index.truncated), max_lines: count(index.max_lines), max_bytes: count(index.max_bytes) };
    }
    async saveLocal(bot_id: string, value: string, expected_hash: string) {
        const row = object(await this.transport("memory.write", { bot_id, text: value, expected_hash }));
        return { hash: text(row.hash) };
    }
    private consume(preview: object, confirm: boolean) {
        if (!confirm)
            throw new Error("confirmation_required");
        const approval = this.approvals.get(preview);
        this.approvals.delete(preview);
        if (approval === undefined)
            throw new Error("approval_not_found");
        if (Date.now() >= approval.expires)
            throw new Error("approval_expired");
        if (JSON.stringify(preview) !== approval.snapshot)
            throw new Error("approval_stale");
    }
}
