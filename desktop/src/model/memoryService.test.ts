import { expect, test } from "bun:test";
import { MemoryPreferencesDraft, MemoryServiceAPI, MemoryServiceViewState, connectionRequest, memoryVectorEditBody, readMemoryConnections, readMemoryHealth, readMemoryOperations, secretPatch, supportsMemoryAction, supportsMemoryAdvancedAction, type MemoryPreferences, type MemoryPreferencesView, type MemoryRequest } from "./memoryService";

const baseline = (): MemoryPreferences => ({
  connection_id: null, auto_recall: false, capture_conversation: false, capture_group_text: false,
  unattended_capture: false, max_capture_deliveries_per_turn: 1,
  recall_budget: { timeout_ms: 2000, max_bytes: 16384, max_results: 8, max_context_chars: 4000 },
});

function fixture() {
  const calls: MemoryRequest[] = [];
  return { calls, api: new MemoryServiceAPI(async (method, params) => { calls.push({ method, params } as MemoryRequest); return null; }) };
}

test("new consent drafts never opt in to capture, groups, or background spending", () => {
  const draft = new MemoryPreferencesDraft("bot-a");
  expect(draft.request().params).toMatchObject({ bot_id: "bot-a", connection_id: null, auto_recall: false, capture_conversation: false, capture_group_text: false, unattended_capture: false });
});

test("plaintext approval is target-bound and enabling recall cannot silently send a query", async () => {
  const { api, calls } = fixture();
  const draft = new MemoryPreferencesDraft("bot-a");
  draft.connectionID = "remote-a";
  draft.autoRecall = true;
  await expect(api.savePreferences(draft)).rejects.toThrow("plaintext_consent_required");
  expect(calls).toEqual([]);
  draft.approveRemotePlaintext();
  await api.savePreferences(draft);
  expect(calls[0]).toEqual({ method: "memory.preferences.set", params: { ...baseline(), bot_id: "bot-a", connection_id: "remote-a", auto_recall: true } });
  draft.connectionID = "remote-b";
  await expect(api.savePreferences(draft)).rejects.toThrow("plaintext_consent_required");
  expect(calls).toHaveLength(1);
});

test("group capture needs a separate approval and only future capture is requested", () => {
  const draft = new MemoryPreferencesDraft("bot-a");
  draft.connectionID = "remote-a";
  draft.captureConversation = true;
  draft.captureGroupText = true;
  draft.approveRemotePlaintext();
  expect(() => draft.request()).toThrow("group_consent_required");
  draft.approveGroupCapture();
  const params = draft.request().params;
  expect(params).toMatchObject({ capture_conversation: true, capture_group_text: true });
  expect(Object.keys(params).sort()).toEqual(["auto_recall", "bot_id", "capture_conversation", "capture_group_text", "connection_id", "max_capture_deliveries_per_turn", "recall_budget", "unattended_capture"].sort());
});

test("changing a draft target invalidates approvals even when returning to the old target", () => {
  const draft = new MemoryPreferencesDraft("bot-a");
  draft.connectionID = "a"; draft.captureConversation = true; draft.approveRemotePlaintext();
  draft.connectionID = "b"; draft.connectionID = "a";
  expect(() => draft.request()).toThrow("plaintext_consent_required");
});

test("editing existing approved preferences does not mutate loaded budgets", () => {
  const saved = { ...baseline(), connection_id: "a", capture_conversation: true, capture_group_text: true };
  const draft = new MemoryPreferencesDraft("bot-a", saved);
  draft.recallBudget.max_results = 4;
  expect(saved.recall_budget.max_results).toBe(8);
  expect(draft.request().params).toMatchObject({ capture_conversation: true, capture_group_text: true });
  draft.captureConversation = false;
  draft.captureGroupText = false;
  expect(draft.request().params).toMatchObject({ capture_conversation: false, capture_group_text: false });
});

test("background spending stays unavailable without a frozen enforceable cost policy", async () => {
  const { api, calls } = fixture();
  const draft = new MemoryPreferencesDraft("bot-a");
  draft.connectionID = "a"; draft.captureConversation = true; draft.approveRemotePlaintext();
  draft.unattendedCapture = true;
  await expect(api.savePreferences(draft)).rejects.toThrow("enforced_spending_policy_required");
  expect(calls).toEqual([]);
});

test("invalid budgets and capture caps fail before RPC without clamping consent", () => {
  for (const [key, value] of [["timeout_ms", 5001], ["max_bytes", 32769], ["max_results", 21], ["max_context_chars", 99], ["max_results", 1.5], ["timeout_ms", NaN]] as const) {
    const draft = new MemoryPreferencesDraft("bot-a");
    draft.recallBudget[key] = value;
    expect(() => draft.request()).toThrow("invalid_budget");
  }
  const draft = new MemoryPreferencesDraft("bot-a");
  draft.maxCaptureDeliveriesPerTurn = 5;
  expect(() => draft.request()).toThrow("invalid_capture_cap");
  draft.maxCaptureDeliveriesPerTurn = 4;
  draft.recallBudget = { timeout_ms: 5000, max_bytes: 32768, max_results: 20, max_context_chars: 8000 };
  expect(draft.request().params).toMatchObject({ max_capture_deliveries_per_turn: 4, recall_budget: draft.recallBudget });
});

test("secret patches distinguish keep, clear and exact replacement; validate UTF-8 bytes", () => {
  expect(secretPatch("keep", "ignored")).toEqual({ action: "keep" });
  expect(secretPatch("clear", "ignored")).toEqual({ action: "clear" });
  expect(secretPatch("replace", " exact key ")).toEqual({ action: "replace", value: " exact key " });
  expect(() => secretPatch("replace", "")).toThrow("invalid_secret");
  expect(() => secretPatch("replace", "é".repeat(4097))).toThrow("invalid_secret");
});

test("connection edits whitelist contract fields and never serialize an injected namespace or stored key", () => {
  const input = { id: "c1", backend: "hindsight" as const, name: "Personal", endpoint: "https://memory.example", secret: { action: "keep" as const }, namespace: "other-bank", storedKey: "must-not-send" };
  expect(connectionRequest(input)).toEqual({ method: "memory.connections.set", params: { id: "c1", backend: "hindsight", name: "Personal", endpoint: "https://memory.example", secret: { action: "keep" } } });
});

test("service controls fail closed for absent and future capability values", () => {
  expect(supportsMemoryAction(undefined, "reflect")).toBe(false);
  expect(supportsMemoryAction({ reflect: false, recall: true }, "reflect")).toBe(false);
  expect(supportsMemoryAction({ recall: true }, "recall")).toBe(true);
  expect(supportsMemoryAction({ advanced: ["sessions"] }, "mental_models")).toBe(false);
  expect(supportsMemoryAction({ advanced: ["sessions"] }, "sessions")).toBe(true);
  expect(supportsMemoryAction({ reflect: "true" } as never, "reflect")).toBe(false);
});

test("OpenViking options use exact bot IDs and secret patches, never namespace selectors or raw keys", () => {
  const request = connectionRequest({
    id: "viking", backend: "open_viking", name: "Viking", secret: { action: "keep" },
    options: { backend: "open_viking", bindings: {
      "bot-exact": { mode: "user_key", account_id: "account", user_id: "user", secret: { action: "replace", value: "new-key" }, api_key: "must-not-send" } as never,
    } },
  });
  expect(request.params.options).toEqual({ backend: "open_viking", bindings: {
    "bot-exact": { mode: "user_key", account_id: "account", user_id: "user", secret: { action: "replace", value: "new-key" } },
  } });
  expect(() => connectionRequest({ id: "a", backend: "pgvector", name: "a", secret: { action: "keep" }, options: { backend: "hindsight" } })).toThrow("backend_options_mismatch");
  expect(connectionRequest({ id: "a", backend: "pgvector", name: "a", secret: { action: "keep" } }).params).not.toHaveProperty("options");
});

test("masked connection replies strip unexpected private values and preserve tombstone-free IDs", () => {
  const result = readMemoryConnections({
    schema_version: 1,
    connections: [{ id: "a", revision: { counter: 2, device_id: "device-a" }, backend: "pgvector", name: "Personal", has_secret: true, embedding_profile: null, availability: "supported", reason: null, secret: "must-not-show", endpoint: "postgres://private", options: { secret: "must-not-show" } }],
    embeddings: [], bots: [],
  });
  expect(result.connections[0]).toEqual({ id: "a", revision: { counter: 2, device_id: "device-a" }, backend: "pgvector", name: "Personal", has_secret: true, embedding_profile: null, availability: "supported", reason: null });
  expect(() => readMemoryConnections({ schema_version: 2, connections: [], embeddings: [], bots: [] })).toThrow("invalid_memory_reply");
});

test("health and operation status never equate pending/unknown delivery with stored or erased", () => {
  expect(readMemoryHealth({ status: "setup_required", capabilities: {}, deletion_pending: true })).toEqual({ status: "setup_required", capabilities: { advanced: [] }, deletion_pending: true });
  const health = readMemoryHealth({ status: "degraded", capabilities: { recall: true, advanced: ["sessions", "future_feature"], reflect: false }, deletion_pending: true });
  expect(health.capabilities.advanced).toEqual(["sessions"]);
  expect(supportsMemoryAction(health.capabilities, "reflect")).toBe(false);
  expect(() => readMemoryHealth({ status: "future_ready", capabilities: {}, deletion_pending: false })).toThrow("invalid_memory_reply");
  const operations = readMemoryOperations({ operations: [{ id: "d1", document_id: "doc1", state: "delivery_unknown", operation_id: null, error_code: "lost_reply", document: "private-text" }], deletion: { pending: true, operation_id: null, deletion_epoch: 3 } });
  expect(operations).toEqual({ operations: [{ id: "d1", document_id: "doc1", state: "delivery_unknown", operation_id: null, error_code: "lost_reply" }], deletion: { pending: true, operation_id: null, deletion_epoch: 3 } });
});

test("OpenViking patch removal and replace-all confirmation never silently erase omitted bindings", () => {
  const base = { id: "viking", backend: "open_viking" as const, name: "Viking", secret: { action: "keep" as const } };
  const request = connectionRequest({ ...base, options: { backend: "open_viking", bindings: { "bot-remove": null, "bot-clear": { mode: "user_key", account_id: "account", user_id: "user", secret: { action: "clear" } } } } });
  expect(request.params.options).toEqual({ backend: "open_viking", bindings: { "bot-remove": null, "bot-clear": { mode: "user_key", account_id: "account", user_id: "user", secret: { action: "clear" } } } });
  expect(() => connectionRequest({ ...base, options: { backend: "open_viking", bindings: {}, replace_all: true } })).toThrow("confirmation_required");
  expect(connectionRequest({ ...base, options: { backend: "open_viking", bindings: {}, replace_all: true, confirm_replace_all: true } }).params.options).toEqual({ backend: "open_viking", bindings: {}, replace_all: true, confirm_replace_all: true });
});

test("advanced actions fail closed independently of feature flags, and cannot bypass fenced document deletion", () => {
  expect(supportsMemoryAdvancedAction({ advanced: ["bank_config"] }, "bank_config", "update")).toBe(false);
  expect(supportsMemoryAdvancedAction({ advanced: ["bank_config"], advanced_actions: { bank_config: [] } }, "bank_config", "get")).toBe(false);
  const health = readMemoryHealth({ status: "ready", deletion_pending: false, capabilities: { advanced: ["bank_config", "documents"], advanced_actions: { bank_config: ["get"], documents: ["get", "delete"], future_feature: ["anything"] } } });
  expect(supportsMemoryAdvancedAction(health.capabilities, "bank_config", "get")).toBe(true);
  expect(supportsMemoryAdvancedAction(health.capabilities, "bank_config", "update")).toBe(false);
  expect(supportsMemoryAdvancedAction(health.capabilities, "documents", "delete")).toBe(false);
  expect(health.capabilities.advanced_actions).toEqual({ bank_config: ["get"], documents: ["get"] });
  expect(supportsMemoryAdvancedAction({ advanced: ["bank_config"], advanced_actions: { bank_config: ["bank_selector"] } }, "bank_config", "bank_selector")).toBe(false);
});

const preferenceView = (deletion_epoch = 0): MemoryPreferencesView => ({ ...baseline(), connection_id: "memory-a", consent_revision: { counter: 1, device_id: "fixture" }, deletion_epoch });

test("vector memory edit keeps its negotiated edit verb and exact document_id/text contract", () => {
  const capabilities = { advanced: ["memory_edit" as const], advanced_actions: { memory_edit: ["edit"] } };
  expect(supportsMemoryAdvancedAction(capabilities, "memory_edit", "edit")).toBe(true);
  expect(supportsMemoryAdvancedAction(capabilities, "memory_edit", "update")).toBe(false);
  expect(memoryVectorEditBody("doc-a", "Replacement text")).toEqual({ document_id: "doc-a", text: "Replacement text" });
  expect(() => memoryVectorEditBody("", "text")).toThrow("invalid_memory_edit");
  expect(() => memoryVectorEditBody("doc-a", "é".repeat(16385))).toThrow("invalid_memory_edit");
});

test("authoritative status refresh preserves an unsaved auto-recall draft after durable failed retain", () => {
  const view = new MemoryServiceViewState("bot-a", preferenceView());
  view.draft.autoRecall = true;
  const status = readMemoryOperations({ operations: [{ id: "delivery", document_id: "doc", state: "failed", operation_id: null, error_code: "service_rejected" }], deletion: null });
  view.refreshPreferences(preferenceView());
  expect(status.operations[0]?.state).toBe("failed");
  expect(view.draft.autoRecall).toBe(true);
  expect(view.preferences.auto_recall).toBe(false);
});

test("a deletion fence locks subsequent delete through failed or stale preference refresh", async () => {
  const view = new MemoryServiceViewState("bot-a", preferenceView());
  const admittedEpochs: number[] = [view.beginDeletion()];
  view.recordDeletionEpoch(1);
  const api = new MemoryServiceAPI(async () => { throw new Error("fixture_refresh_failed"); });
  await expect(api.getPreferences("bot-a")).rejects.toThrow("fixture_refresh_failed");
  expect(view.deletionLocked).toBe(true);
  expect(() => admittedEpochs.push(view.beginDeletion())).toThrow("preferences_refresh_required");
  expect(() => view.refreshPreferences(preferenceView(0))).toThrow("stale_preferences");
  expect(view.deletionLocked).toBe(true);
  view.refreshPreferences(preferenceView(1));
  admittedEpochs.push(view.beginDeletion());
  expect(admittedEpochs).toEqual([0, 1]);
});

test("masked availability distinguishes blocked Cloud from eligible local Lance without claiming readiness", () => {
  const base = { id: "lance", revision: { counter: 1, device_id: "fixture" }, backend: "lance_db" as const, name: "Lance", has_secret: true, embedding_profile: null };
  const listing = (connection: unknown) => ({ schema_version: 1, connections: [connection], embeddings: [], bots: [] });
  const cloud = readMemoryConnections(listing({ ...base, availability: "blocked", reason: "lancedb_cloud_transport_unavailable", endpoint: "db://private", path: "/private", key: "secret" })).connections[0];
  expect(cloud).toEqual({ ...base, availability: "blocked", reason: "lancedb_cloud_transport_unavailable" });
  expect(readMemoryConnections(listing({ ...base, availability: "supported", reason: null })).connections[0]).toEqual({ ...base, availability: "supported", reason: null });
  expect(() => readMemoryConnections(listing({ ...base, availability: "ready", reason: null }))).toThrow("invalid_memory_reply");
  expect(() => readMemoryConnections(listing(base))).toThrow("invalid_memory_reply");
  expect(readMemoryHealth({ status: "setup_required", capabilities: {}, deletion_pending: false, reason: "lancedb_cloud_transport_unavailable" })).toEqual({ status: "setup_required", capabilities: { advanced: [] }, deletion_pending: false, reason: "lancedb_cloud_transport_unavailable" });
});
