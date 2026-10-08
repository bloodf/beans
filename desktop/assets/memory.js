(() => {
  var __defProp = Object.defineProperty;
  var __returnValue = (v) => v;
  function __exportSetter(name, newValue) {
    this[name] = __returnValue.bind(null, newValue);
  }
  var __export = (target, all) => {
    for (var name in all)
      __defProp(target, name, {
        get: all[name],
        enumerable: true,
        configurable: true,
        set: __exportSetter.bind(all, name)
      });
  };

  // src/model/memoryService.ts
  var exports_memoryService = {};
  __export(exports_memoryService, {
    MEMORY_ADVANCED_ACTIONS: () => MEMORY_ADVANCED_ACTIONS,
    MemoryPreferencesDraft: () => MemoryPreferencesDraft,
    MemoryServiceAPI: () => MemoryServiceAPI,
    MemoryServiceViewState: () => MemoryServiceViewState,
    connectionRequest: () => connectionRequest,
    memoryVectorEditBody: () => memoryVectorEditBody,
    readMemoryConnections: () => readMemoryConnections,
    readMemoryHealth: () => readMemoryHealth,
    readMemoryOperations: () => readMemoryOperations,
    readMemoryPreferences: () => readMemoryPreferences,
    secretPatch: () => secretPatch,
    supportsMemoryAction: () => supportsMemoryAction,
    supportsMemoryAdvancedAction: () => supportsMemoryAdvancedAction
  });
  var MEMORY_ADVANCED_ACTIONS = {
    bank_profile: ["get", "update", "reset"],
    bank_config: ["get", "update", "reset"],
    directives: ["list", "get", "create", "update", "delete"],
    mental_models: ["list", "get", "create", "update", "refresh", "history", "delete"],
    mental_model_history: ["list", "get"],
    observations: ["list", "scopes", "clear", "clear_derived"],
    memory_edit: ["get", "history", "update", "edit"],
    memory_invalidate: ["get", "history", "update"],
    memory_restore: ["get", "history", "update"],
    documents: ["list", "get", "chunks"],
    sessions: ["list", "get", "create", "add_message", "commit", "delete"],
    resources: ["list", "read", "add", "delete"],
    tasks: ["list", "get", "cancel", "delete_record"]
  };
  function secretPatch(action, value = "") {
    if (action !== "replace")
      return { action };
    const bytes = new TextEncoder().encode(value).length;
    if (bytes === 0 || bytes > 8192)
      throw new Error("invalid_secret");
    return { action, value };
  }
  function connectionRequest(edit) {
    const params = {
      id: edit.id,
      backend: edit.backend,
      name: edit.name,
      secret: edit.secret.action === "replace" ? secretPatch("replace", edit.secret.value) : secretPatch(edit.secret.action)
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
          if (edit.options.role !== undefined)
            params.options.role = edit.options.role;
          break;
        case "lance_db":
          params.options = { backend: "lance_db", region: edit.options.region };
          break;
        case "open_viking": {
          if (edit.options.replace_all === true && edit.options.confirm_replace_all !== true)
            throw new Error("confirmation_required");
          const bindings = Object.create(null);
          for (const [botID, binding] of Object.entries(edit.options.bindings)) {
            if (binding === null) {
              bindings[botID] = null;
              continue;
            }
            bindings[botID] = {
              mode: binding.mode,
              account_id: binding.account_id,
              user_id: binding.user_id,
              secret: binding.secret.action === "replace" ? secretPatch("replace", binding.secret.value) : secretPatch(binding.secret.action)
            };
          }
          params.options = { backend: "open_viking", bindings };
          if (edit.options.replace_all !== undefined)
            params.options.replace_all = edit.options.replace_all;
          if (edit.options.confirm_replace_all !== undefined)
            params.options.confirm_replace_all = edit.options.confirm_replace_all;
          break;
        }
        default:
          throw new Error("invalid_backend_options");
      }
    }
    return { method: "memory.connections.set", params };
  }
  function supportsMemoryAction(capabilities, action) {
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
      default:
        return capabilities.advanced?.includes(action) === true;
    }
  }
  function supportsMemoryAdvancedAction(capabilities, feature, action) {
    if (feature === "documents" && action === "delete")
      return false;
    return capabilities?.advanced?.includes(feature) === true && MEMORY_ADVANCED_ACTIONS[feature].includes(action) && capabilities.advanced_actions?.[feature]?.includes(action) === true;
  }
  function memoryVectorEditBody(documentID, text) {
    if (!documentID || documentID.length > 256 || /[\u0000-\u001f\u007f]/.test(documentID) || !text || new TextEncoder().encode(text).length > 32768)
      throw new Error("invalid_memory_edit");
    return { document_id: documentID, text };
  }
  var initialPreferences = () => ({
    connection_id: null,
    auto_recall: false,
    capture_conversation: false,
    capture_group_text: false,
    unattended_capture: false,
    max_capture_deliveries_per_turn: 1,
    recall_budget: { timeout_ms: 2000, max_bytes: 16384, max_results: 8, max_context_chars: 4000 }
  });
  function validateBudget(budget) {
    for (const [value, min, max] of [
      [budget.timeout_ms, 1, 5000],
      [budget.max_bytes, 1, 32768],
      [budget.max_results, 1, 20],
      [budget.max_context_chars, 100, 8000]
    ]) {
      if (value === undefined || min === undefined || max === undefined || !Number.isSafeInteger(value) || value < min || value > max)
        throw new Error("invalid_budget");
    }
  }

  class MemoryPreferencesDraft {
    botID;
    connection;
    original;
    originalConsentValid = true;
    plaintextApproval;
    groupApproval;
    autoRecall;
    captureConversation;
    captureGroupText;
    unattendedCapture;
    maxCaptureDeliveriesPerTurn;
    recallBudget;
    constructor(botID, saved = initialPreferences()) {
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
    get connectionID() {
      return this.connection;
    }
    set connectionID(value) {
      if (value === this.connection)
        return;
      this.connection = value;
      this.plaintextApproval = this.groupApproval = undefined;
      this.originalConsentValid = false;
    }
    consentTarget() {
      return JSON.stringify([this.botID, this.connection, this.autoRecall, this.captureConversation, this.captureGroupText, this.maxCaptureDeliveriesPerTurn]);
    }
    approveRemotePlaintext() {
      this.plaintextApproval = this.consentTarget();
    }
    approveGroupCapture() {
      this.groupApproval = this.consentTarget();
    }
    request() {
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
      const needsPlaintext = this.autoRecall && !(retainsOriginalConsent && this.original.auto_recall) || this.captureConversation && !(retainsOriginalConsent && this.original.capture_conversation && this.maxCaptureDeliveriesPerTurn <= this.original.max_capture_deliveries_per_turn);
      if (needsPlaintext && this.plaintextApproval !== this.consentTarget())
        throw new Error("plaintext_consent_required");
      if (this.captureGroupText && !(retainsOriginalConsent && this.original.capture_group_text) && this.groupApproval !== this.consentTarget())
        throw new Error("group_consent_required");
      return {
        method: "memory.preferences.set",
        params: {
          bot_id: this.botID,
          connection_id: this.connection,
          auto_recall: this.autoRecall,
          capture_conversation: this.captureConversation,
          capture_group_text: this.captureGroupText,
          unattended_capture: false,
          max_capture_deliveries_per_turn: this.maxCaptureDeliveriesPerTurn,
          recall_budget: { ...this.recallBudget }
        }
      };
    }
  }

  class MemoryServiceAPI {
    transport;
    constructor(transport) {
      this.transport = transport;
    }
    send(request) {
      return this.transport(request.method, request.params);
    }
    async listConnections() {
      return readMemoryConnections(await this.send({ method: "memory.connections.list", params: {} }));
    }
    setConnection(edit) {
      return this.send(connectionRequest(edit));
    }
    disconnectConnection(id) {
      return this.send({ method: "memory.connections.disconnect", params: { id } });
    }
    async getPreferences(botID) {
      return readMemoryPreferences(await this.send({ method: "memory.preferences.get", params: { bot_id: botID } }));
    }
    savePreferences(draft) {
      return Promise.resolve().then(() => this.send(draft.request()));
    }
    async health(botID) {
      return readMemoryHealth(await this.send({ method: "memory.service.health", params: { bot_id: botID } }));
    }
    async operations(botID) {
      return readMemoryOperations(await this.send({ method: "memory.operations.list", params: { bot_id: botID } }));
    }
  }
  var advancedFeatures = ["bank_profile", "bank_config", "directives", "mental_models", "mental_model_history", "observations", "memory_edit", "memory_invalidate", "memory_restore", "documents", "sessions", "resources", "tasks"];
  var operationStates = ["queued", "submitted", "processing", "completed", "failed", "delivery_unknown"];
  var backendKinds = ["hindsight", "open_viking", "pgvector", "lance_db"];
  function object(value) {
    if (!value || typeof value !== "object" || Array.isArray(value))
      throw new Error("invalid_memory_reply");
    return value;
  }
  function text(value) {
    if (typeof value !== "string")
      throw new Error("invalid_memory_reply");
    return value;
  }
  function nullableText(value) {
    if (value === null)
      return null;
    return text(value);
  }
  function flag(value) {
    if (typeof value !== "boolean")
      throw new Error("invalid_memory_reply");
    return value;
  }
  function count(value) {
    if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0)
      throw new Error("invalid_memory_reply");
    return value;
  }
  function array(value) {
    if (!Array.isArray(value))
      throw new Error("invalid_memory_reply");
    return value;
  }
  function revision(value) {
    const row = object(value);
    return { counter: count(row.counter), device_id: text(row.device_id) };
  }
  function readMemoryPreferences(value) {
    const row = object(value), budget = object(row.recall_budget);
    const recall_budget = { timeout_ms: count(budget.timeout_ms), max_bytes: count(budget.max_bytes), max_results: count(budget.max_results), max_context_chars: count(budget.max_context_chars) };
    validateBudget(recall_budget);
    const cap = count(row.max_capture_deliveries_per_turn);
    if (cap > 4)
      throw new Error("invalid_memory_reply");
    return {
      connection_id: nullableText(row.connection_id),
      auto_recall: flag(row.auto_recall),
      capture_conversation: flag(row.capture_conversation),
      capture_group_text: flag(row.capture_group_text),
      unattended_capture: flag(row.unattended_capture),
      max_capture_deliveries_per_turn: cap,
      recall_budget,
      consent_revision: revision(row.consent_revision),
      deletion_epoch: count(row.deletion_epoch)
    };
  }
  function readMemoryConnections(value) {
    const row = object(value);
    if (row.schema_version !== 1)
      throw new Error("invalid_memory_reply");
    return {
      schema_version: 1,
      connections: array(row.connections).map((value) => {
        const c = object(value), backend = text(c.backend);
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
      })
    };
  }
  function readMemoryHealth(value) {
    const row = object(value), status = text(row.status);
    if (status !== "ready" && status !== "degraded" && status !== "setup_required")
      throw new Error("invalid_memory_reply");
    const supplied = object(row.capabilities), capabilities = {};
    for (const name of ["retain", "recall", "inspect", "delete_document", "clear", "operation_status", "cancel_operation", "reflect", "idempotent_retain", "write_fence"]) {
      if (supplied[name] !== undefined)
        capabilities[name] = flag(supplied[name]);
    }
    if (supplied.advanced_actions !== undefined) {
      const negotiated = object(supplied.advanced_actions);
      capabilities.advanced_actions = {};
      for (const feature of advancedFeatures) {
        if (negotiated[feature] !== undefined)
          capabilities.advanced_actions[feature] = array(negotiated[feature]).map(text).filter((action) => !(feature === "documents" && action === "delete"));
      }
    }
    capabilities.advanced = supplied.advanced === undefined ? [] : array(supplied.advanced).map(text).filter((name) => advancedFeatures.includes(name));
    return { status, capabilities, deletion_pending: flag(row.deletion_pending), ...row.reason === undefined ? {} : { reason: nullableText(row.reason) } };
  }
  function readMemoryOperations(value) {
    const row = object(value), deletion = row.deletion === null ? null : object(row.deletion);
    return {
      operations: array(row.operations).map((value) => {
        const o = object(value), state = text(o.state);
        if (!operationStates.includes(state))
          throw new Error("invalid_memory_reply");
        return { id: text(o.id), document_id: text(o.document_id), state, operation_id: nullableText(o.operation_id), error_code: nullableText(o.error_code) };
      }),
      deletion: deletion === null ? null : { pending: flag(deletion.pending), operation_id: nullableText(deletion.operation_id), deletion_epoch: count(deletion.deletion_epoch) }
    };
  }

  class MemoryServiceViewState {
    draft;
    latest;
    minimumDeletionEpoch;
    needsDeletionRefresh = false;
    constructor(botID, preferences) {
      this.latest = structuredClone(preferences);
      this.minimumDeletionEpoch = preferences.deletion_epoch;
      this.draft = new MemoryPreferencesDraft(botID, preferences);
    }
    get preferences() {
      return this.latest;
    }
    get deletionLocked() {
      return this.needsDeletionRefresh;
    }
    beginDeletion() {
      if (this.needsDeletionRefresh)
        throw new Error("preferences_refresh_required");
      this.needsDeletionRefresh = true;
      return this.latest.deletion_epoch;
    }
    recordDeletionEpoch(epoch) {
      if (!Number.isSafeInteger(epoch) || epoch < this.latest.deletion_epoch)
        throw new Error("invalid_memory_reply");
      this.minimumDeletionEpoch = Math.max(this.minimumDeletionEpoch, epoch);
    }
    refreshPreferences(preferences) {
      if (preferences.deletion_epoch < this.minimumDeletionEpoch)
        throw new Error("stale_preferences");
      this.latest = structuredClone(preferences);
      this.minimumDeletionEpoch = preferences.deletion_epoch;
      this.needsDeletionRefresh = false;
    }
  }

  // src/model/memorySetup.ts
  var exports_memorySetup = {};
  __export(exports_memorySetup, {
    BotSetupApproval: () => BotSetupApproval,
    LocalAssetApproval: () => LocalAssetApproval,
    MemorySetupAPI: () => MemorySetupAPI,
    readBotSetupPreview: () => readBotSetupPreview,
    readLocalAssetPreview: () => readLocalAssetPreview,
    setupPreviewRequest: () => setupPreviewRequest
  });
  function row(value) {
    if (!value || typeof value !== "object" || Array.isArray(value))
      throw new Error("invalid_setup_reply");
    return value;
  }
  function text2(value) {
    if (typeof value !== "string" || !value)
      throw new Error("invalid_setup_reply");
    return value;
  }
  function integer(value, min = 0, max = Number.MAX_SAFE_INTEGER) {
    if (typeof value !== "number" || !Number.isSafeInteger(value) || value < min || value > max)
      throw new Error("invalid_setup_reply");
    return value;
  }
  function flag2(value) {
    if (typeof value !== "boolean")
      throw new Error("invalid_setup_reply");
    return value;
  }
  function revision2(value) {
    const v = row(value);
    return { counter: integer(v.counter), device_id: text2(v.device_id) };
  }
  function hash(value) {
    const v = text2(value);
    if (!/^[a-f0-9]{64}$/.test(v))
      throw new Error("invalid_setup_reply");
    return v;
  }
  function assets(value) {
    if (!Array.isArray(value) || value.length !== 3)
      throw new Error("invalid_asset_plan");
    const found = new Set;
    const result = value.map((item) => {
      const a = row(item), kind = text2(a.kind), source = row(a.source);
      if (!["runtime", "model", "tokenizer"].includes(kind) || found.has(kind))
        throw new Error("invalid_asset_plan");
      found.add(kind);
      let boundSource;
      if (source.kind === "supplied")
        boundSource = { kind: "supplied", path: text2(source.path) };
      else if (source.kind === "download") {
        const url = text2(source.url);
        let parsed;
        try {
          parsed = new URL(url);
        } catch {
          throw new Error("invalid_asset_source");
        }
        if (parsed.protocol !== "https:" || parsed.username || parsed.password || parsed.hash)
          throw new Error("invalid_asset_source");
        boundSource = { kind: "download", url };
      } else
        throw new Error("invalid_asset_source");
      return { kind, source: boundSource, license: text2(a.license), bytes: integer(a.bytes, 1), sha256: hash(a.sha256) };
    });
    return result;
  }
  function setupPreviewRequest(request) {
    switch (request.method) {
      case "memory.embeddings.local.preview":
        return { method: request.method, params: { runner_id: request.params.runner_id, profile_id: request.params.profile_id, profile_revision: revision2(request.params.profile_revision), plan: { assets: assets(request.params.plan.assets) } } };
      case "memory.pgvector.initialize.preview":
        return { method: request.method, params: { bot_id: request.params.bot_id } };
      case "memory.lance.binding.preview":
        return { method: request.method, params: { bot_id: request.params.bot_id, directory: request.params.directory, create: request.params.create } };
      case "memory.lance.export.preview":
      case "memory.lance.import.preview":
        return { method: request.method, params: { bot_id: request.params.bot_id, path: request.params.path } };
    }
  }
  function readLocalAssetPreview(value) {
    const p = row(value), approvedAssets = assets(p.assets), total = integer(p.total_bytes, 1);
    if (p.expires_in_seconds !== 600 || approvedAssets.reduce((sum, a) => sum + a.bytes, 0) !== total)
      throw new Error("invalid_setup_reply");
    return { preview_token: text2(p.preview_token), preview_digest: hash(p.preview_digest), expires_in_seconds: 600, runner_id: text2(p.runner_id), profile_id: text2(p.profile_id), profile_revision: revision2(p.profile_revision), total_bytes: total, assets: approvedAssets };
  }
  function readBotSetupPreview(value, expectedAction) {
    const p = row(value), d = row(p.details);
    if (p.action !== expectedAction)
      throw new Error("approval_action_mismatch");
    if (p.profile_id !== null || p.profile_revision !== null)
      throw new Error("invalid_setup_reply");
    const base = { token: text2(p.token), expires_at: integer(p.expires_at), runner_id: text2(p.runner_id), bot_id: text2(p.bot_id), connection_revision: revision2(p.connection_revision), profile_id: null, profile_revision: null };
    if (expectedAction === "memory.pgvector.initialize.preview") {
      const t = row(d.target), r = row(d.readiness);
      return { ...base, action: expectedAction, details: {
        target: { database: text2(t.database), database_oid: integer(t.database_oid, 0, 4294967295), role: text2(t.role), server_address: t.server_address === null ? null : text2(t.server_address), server_port: t.server_port === null ? null : integer(t.server_port, -2147483648, 2147483647), server_version: text2(t.server_version), session_pid: integer(t.session_pid, -2147483648, 2147483647) },
        readiness: { ready: flag2(r.ready), extension_version: r.extension_version === null ? null : text2(r.extension_version), extension_schema: r.extension_schema === null ? null : text2(r.extension_schema), schema_exists: flag2(r.schema_exists), can_create_schema: flag2(r.can_create_schema), can_create_tables: flag2(r.can_create_tables), can_install_extension: flag2(r.can_install_extension) },
        schema: text2(d.schema),
        sql: text2(d.sql)
      } };
    }
    if (expectedAction === "memory.lance.binding.preview") {
      if (d.table !== "beans_memory_v1")
        throw new Error("invalid_setup_reply");
      return { ...base, action: expectedAction, details: { directory: text2(d.directory), create: flag2(d.create), table: "beans_memory_v1", namespace: hash(d.namespace) } };
    }
    const space = row(d.space), distance = text2(space.distance);
    if (distance !== "cosine" && distance !== "dot" && distance !== "euclidean")
      throw new Error("invalid_setup_reply");
    return { ...base, action: expectedAction, details: { path: text2(d.path), namespace: hash(d.namespace), space: { fingerprint: hash(space.fingerprint), dimensions: integer(space.dimensions, 1, 65536), distance, generation: integer(space.generation) }, documents: integer(d.documents, 0, 1000), sha256: hash(d.sha256) } };
  }
  var applyMethod = {
    "memory.pgvector.initialize.preview": "memory.pgvector.initialize.apply",
    "memory.lance.binding.preview": "memory.lance.binding.apply",
    "memory.lance.export.preview": "memory.lance.export.apply",
    "memory.lance.import.preview": "memory.lance.import.apply"
  };
  function freezePreview(value) {
    if (value !== null && typeof value === "object") {
      for (const child of Object.values(value))
        freezePreview(child);
      Object.freeze(value);
    }
    return value;
  }

  class BotSetupApproval {
    snapshot;
    get preview() {
      return this.snapshot;
    }
    used = false;
    target;
    token;
    deadline;
    method;
    constructor(preview) {
      this.snapshot = freezePreview(structuredClone(preview));
      this.target = { runner_id: this.preview.runner_id, bot_id: this.preview.bot_id, connection_revision: { ...this.preview.connection_revision } };
      this.token = this.preview.token;
      this.deadline = this.preview.expires_at * 1000;
      this.method = applyMethod[this.preview.action];
    }
    applyRequest(confirm, current, nowMS = Date.now()) {
      if (!confirm)
        throw new Error("confirmation_required");
      if (this.used)
        throw new Error("approval_not_found");
      if (!Number.isFinite(nowMS) || nowMS >= this.deadline)
        throw new Error("approval_expired");
      if (current.bot_id !== this.target.bot_id || current.runner_id !== this.target.runner_id || current.connection_revision.counter !== this.target.connection_revision.counter || current.connection_revision.device_id !== this.target.connection_revision.device_id)
        throw new Error("approval_stale");
      this.used = true;
      return { method: this.method, params: { bot_id: this.target.bot_id, token: this.token, confirm: true } };
    }
  }

  class LocalAssetApproval {
    snapshot;
    get preview() {
      return this.snapshot;
    }
    used = false;
    target;
    token;
    digest;
    deadline;
    constructor(preview, receivedAtMS = Date.now()) {
      this.snapshot = freezePreview(structuredClone(preview));
      this.target = { runner_id: this.preview.runner_id, profile_id: this.preview.profile_id, profile_revision: { ...this.preview.profile_revision } };
      this.token = this.preview.preview_token;
      this.digest = this.preview.preview_digest;
      this.deadline = receivedAtMS + this.preview.expires_in_seconds * 1000;
    }
    applyRequest(confirm, current, nowMS = Date.now()) {
      if (!confirm)
        throw new Error("confirmation_required");
      if (this.used)
        throw new Error("approval_not_found");
      if (!Number.isFinite(nowMS) || nowMS >= this.deadline)
        throw new Error("approval_expired");
      if (current.runner_id !== this.target.runner_id || current.profile_id !== this.target.profile_id || current.profile_revision.counter !== this.target.profile_revision.counter || current.profile_revision.device_id !== this.target.profile_revision.device_id)
        throw new Error("approval_stale");
      this.used = true;
      return { method: "memory.embeddings.local.apply", params: { runner_id: this.target.runner_id, preview_token: this.token, preview_digest: this.digest, confirm: true } };
    }
  }

  class MemorySetupAPI {
    transport;
    constructor(transport) {
      this.transport = transport;
    }
    async preview(request) {
      const safe = setupPreviewRequest(request), response = await this.transport(safe.method, safe.params);
      if (safe.method === "memory.embeddings.local.preview") {
        const preview = readLocalAssetPreview(response);
        if (preview.runner_id !== safe.params.runner_id || preview.profile_id !== safe.params.profile_id || preview.profile_revision.counter !== safe.params.profile_revision.counter || preview.profile_revision.device_id !== safe.params.profile_revision.device_id)
          throw new Error("approval_stale");
        return preview;
      }
      const preview = readBotSetupPreview(response, safe.method);
      if (preview.bot_id !== safe.params.bot_id)
        throw new Error("approval_stale");
      return preview;
    }
    async localStatus(runner_id, profile_id) {
      const reply = row(await this.transport("memory.embeddings.local.status", { runner_id, profile_id }));
      if (reply.status !== "ready" && reply.status !== "setup_required" && reply.status !== "runtime_unavailable")
        throw new Error("invalid_setup_reply");
      return { status: reply.status };
    }
    async applyLocal(approval, confirm, current, nowMS = Date.now()) {
      return this.apply(approval.applyRequest(confirm, current, nowMS));
    }
    async applyBot(approval, confirm, current, nowMS = Date.now()) {
      return this.apply(approval.applyRequest(confirm, current, nowMS));
    }
    async apply(request) {
      const reply = row(await this.transport(request.method, request.params));
      switch (request.method) {
        case "memory.embeddings.local.apply":
          if (reply.installed !== true || reply.status !== "ready" && reply.status !== "runtime_unavailable")
            throw new Error("invalid_setup_reply");
          return { installed: true, status: reply.status };
        case "memory.pgvector.initialize.apply":
          if (reply.initialized !== true)
            throw new Error("invalid_setup_reply");
          return { initialized: true };
        case "memory.lance.binding.apply":
          if (reply.status !== "ready")
            throw new Error("invalid_setup_reply");
          return { status: "ready" };
        case "memory.lance.export.apply":
          if (reply.exported !== true)
            throw new Error("invalid_setup_reply");
          return { exported: true, bytes: integer(reply.bytes, 0, 8 * 1024 * 1024) };
        case "memory.lance.import.apply":
          if (reply.imported !== true)
            throw new Error("invalid_setup_reply");
          return { imported: true };
      }
    }
  }

  // src/model/nativeMemory.ts
  Object.assign(globalThis, { BeansMemory: { ...exports_memoryService, ...exports_memorySetup } });
})();
