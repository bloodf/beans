import Foundation

// Memory service models are independent of the local MEMORY.md editor and AppStore.
// Common replies/options follow the frozen memory-ui-contract; setup wiring remains core-owned.
enum MemoryServiceBackend: String, Codable { case hindsight, openViking = "open_viking", pgvector, lanceDB = "lance_db" }
enum MemoryServiceDraftError: String, Error {
    case invalidSecret = "invalid_secret", invalidBudget = "invalid_budget", invalidCaptureCap = "invalid_capture_cap"
    case plaintextConsentRequired = "plaintext_consent_required", groupConsentRequired = "group_consent_required"
    case groupCaptureRequiresConversation = "group_capture_requires_conversation", connectionRequired = "connection_required"
    case enforcedSpendingPolicyRequired = "enforced_spending_policy_required"
    case backendOptionsMismatch = "backend_options_mismatch"
    case confirmationRequired = "confirmation_required"
}

enum MemoryServiceSecretPatch: Encodable, CustomStringConvertible, CustomDebugStringConvertible {
    case keep, replace(String), clear
    private enum Keys: String, CodingKey { case action, value }
    var description: String {
        switch self { case .keep: return "keep"; case .replace: return "replace(<redacted>)"; case .clear: return "clear" }
    }
    var debugDescription: String { description }
    func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: Keys.self)
        switch self {
        case .keep: try values.encode("keep", forKey: .action)
        case .clear: try values.encode("clear", forKey: .action)
        case let .replace(value):
            guard !value.isEmpty, value.utf8.count <= 8192 else { throw MemoryServiceDraftError.invalidSecret }
            try values.encode("replace", forKey: .action)
            try values.encode(value, forKey: .value)
        }
    }
}

enum MemoryServiceTextPatch { case keep, set(String), clear }
enum MemoryServiceConnectionOptions: Encodable {
    case hindsight, pgvector(schema: String, role: String? = nil), lanceDB(region: String?)
    case openViking(bindings: [String: MemoryServiceOpenVikingBindingPatch], replaceAll: Bool = false, confirmReplaceAll: Bool = false)
    var backend: MemoryServiceBackend {
        switch self {
        case .hindsight: return .hindsight
        case .pgvector: return .pgvector
        case .lanceDB: return .lanceDB
        case .openViking: return .openViking
        }
    }
    private enum Keys: String, CodingKey {
        case backend, schema, role, region, bindings
        case replaceAll = "replace_all", confirmReplaceAll = "confirm_replace_all"
    }
    func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: Keys.self)
        try values.encode(backend, forKey: .backend)
        switch self {
        case .hindsight: break
        case let .pgvector(schema, role):
            try values.encode(schema, forKey: .schema)
            try values.encode(role, forKey: .role)
        case let .lanceDB(region): try values.encode(region, forKey: .region)
        case let .openViking(bindings, replaceAll, confirmReplaceAll):
            guard !replaceAll || confirmReplaceAll else { throw MemoryServiceDraftError.confirmationRequired }
            try values.encode(bindings, forKey: .bindings)
            try values.encode(replaceAll, forKey: .replaceAll)
            try values.encode(confirmReplaceAll, forKey: .confirmReplaceAll)
        }
    }
}
enum MemoryServiceOpenVikingBindingPatch: Encodable {
    case set(MemoryServiceOpenVikingBinding), remove
    func encode(to encoder: Encoder) throws {
        switch self {
        case let .set(binding): try binding.encode(to: encoder)
        case .remove: var value = encoder.singleValueContainer(); try value.encodeNil()
        }
    }
}
struct MemoryServiceOpenVikingBinding: Encodable {
    enum Mode: String, Encodable { case userKey = "user_key", trustedGateway = "trusted_gateway" }
    var mode: Mode
    var accountID: String
    var userID: String
    var secret: MemoryServiceSecretPatch
    private enum CodingKeys: String, CodingKey { case mode, accountID = "account_id", userID = "user_id", secret }
}

struct MemoryServiceConnectionEdit: Encodable {
    let id: String
    let backend: MemoryServiceBackend
    var name: String
    var endpoint: MemoryServiceTextPatch = .keep
    var secret: MemoryServiceSecretPatch = .keep
    var embeddingProfile: MemoryServiceTextPatch = .keep
    var allowInsecureHTTP: Bool? = nil
    var options: MemoryServiceConnectionOptions? = nil
    private enum CodingKeys: String, CodingKey {
        case id, backend, name, endpoint, secret, options
        case embeddingProfile = "embedding_profile", allowInsecureHTTP = "allow_insecure_http"
    }
    func encode(to encoder: Encoder) throws {
        guard options == nil || options?.backend == backend else { throw MemoryServiceDraftError.backendOptionsMismatch }
        var values = encoder.container(keyedBy: CodingKeys.self)
        try values.encode(id, forKey: .id)
        try values.encode(backend, forKey: .backend)
        try values.encode(name, forKey: .name)
        try values.encode(secret, forKey: .secret)
        try values.encodeIfPresent(allowInsecureHTTP, forKey: .allowInsecureHTTP)
        try values.encodeIfPresent(options, forKey: .options)
        switch endpoint {
        case .keep: break
        case let .set(value): try values.encode(value, forKey: .endpoint)
        case .clear: try values.encodeNil(forKey: .endpoint)
        }
        switch embeddingProfile {
        case .keep: break
        case let .set(value): try values.encode(value, forKey: .embeddingProfile)
        case .clear: try values.encodeNil(forKey: .embeddingProfile)
        }
    }
}

struct MemoryServiceRecallBudget: Codable, Equatable {
    var timeoutMS: UInt64 = 2000
    var maxBytes: Int = 16384
    var maxResults: Int = 8
    var maxContextChars: Int = 4000
    private enum CodingKeys: String, CodingKey {
        case timeoutMS = "timeout_ms", maxBytes = "max_bytes", maxResults = "max_results", maxContextChars = "max_context_chars"
    }
    func validate() throws {
        guard (1...5000).contains(timeoutMS), (1...32768).contains(maxBytes), (1...20).contains(maxResults),
              (100...8000).contains(maxContextChars) else { throw MemoryServiceDraftError.invalidBudget }
    }
}

struct MemoryServicePreferences: Encodable, Equatable {
    var connectionID: String? = nil
    var autoRecall = false
    var captureConversation = false
    var captureGroupText = false
    var unattendedCapture = false
    var maxCaptureDeliveriesPerTurn = 1
    var recallBudget = MemoryServiceRecallBudget()
    private enum Keys: String, CodingKey {
        case connectionID = "connection_id", autoRecall = "auto_recall", captureConversation = "capture_conversation"
        case captureGroupText = "capture_group_text", unattendedCapture = "unattended_capture"
        case maxCaptureDeliveriesPerTurn = "max_capture_deliveries_per_turn", recallBudget = "recall_budget"
    }
    func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: Keys.self)
        // Explicit null removes the binding; omission must not accidentally keep it.
        try values.encode(connectionID, forKey: .connectionID)
        try values.encode(autoRecall, forKey: .autoRecall)
        try values.encode(captureConversation, forKey: .captureConversation)
        try values.encode(captureGroupText, forKey: .captureGroupText)
        try values.encode(unattendedCapture, forKey: .unattendedCapture)
        try values.encode(maxCaptureDeliveriesPerTurn, forKey: .maxCaptureDeliveriesPerTurn)
        try values.encode(recallBudget, forKey: .recallBudget)
    }
}

struct MemoryServicePreferencesDraft {
    let botID: String
    private let original: MemoryServicePreferences
    private var originalConsentValid = true
    private var plaintextApproval: ConsentTarget?
    private var groupApproval: ConsentTarget?
    var connectionID: String? {
        didSet {
            if connectionID != oldValue {
                plaintextApproval = nil; groupApproval = nil; originalConsentValid = false
            }
        }
    }
    var autoRecall: Bool
    var captureConversation: Bool
    var captureGroupText: Bool
    var unattendedCapture: Bool
    var maxCaptureDeliveriesPerTurn: Int
    var recallBudget: MemoryServiceRecallBudget

    init(botID: String, saved: MemoryServicePreferences = .init()) {
        self.botID = botID; original = saved
        connectionID = saved.connectionID; autoRecall = saved.autoRecall
        captureConversation = saved.captureConversation; captureGroupText = saved.captureGroupText
        unattendedCapture = saved.unattendedCapture; maxCaptureDeliveriesPerTurn = saved.maxCaptureDeliveriesPerTurn
        recallBudget = saved.recallBudget
    }
    private struct ConsentTarget: Equatable {
        let botID: String, connectionID: String?
        let autoRecall: Bool, captureConversation: Bool, captureGroupText: Bool
        let maxCaptureDeliveriesPerTurn: Int
    }
    private var consentTarget: ConsentTarget {
        .init(botID: botID, connectionID: connectionID, autoRecall: autoRecall, captureConversation: captureConversation,
              captureGroupText: captureGroupText, maxCaptureDeliveriesPerTurn: maxCaptureDeliveriesPerTurn)
    }
    mutating func approveRemotePlaintext() { plaintextApproval = consentTarget }
    mutating func approveGroupCapture() { groupApproval = consentTarget }

    func request() throws -> MemoryServiceRequest {
        guard !unattendedCapture else { throw MemoryServiceDraftError.enforcedSpendingPolicyRequired }
        guard (0...4).contains(maxCaptureDeliveriesPerTurn) else { throw MemoryServiceDraftError.invalidCaptureCap }
        try recallBudget.validate()
        guard !captureGroupText || captureConversation else { throw MemoryServiceDraftError.groupCaptureRequiresConversation }
        guard !(autoRecall || captureConversation) || !(connectionID ?? "").isEmpty else { throw MemoryServiceDraftError.connectionRequired }
        let retainsOriginalConsent = originalConsentValid && connectionID == original.connectionID
        let needsPlaintext = (autoRecall && !(retainsOriginalConsent && original.autoRecall)) ||
            (captureConversation && !(retainsOriginalConsent && original.captureConversation && maxCaptureDeliveriesPerTurn <= original.maxCaptureDeliveriesPerTurn))
        guard !needsPlaintext || plaintextApproval == consentTarget else { throw MemoryServiceDraftError.plaintextConsentRequired }
        guard !captureGroupText || (retainsOriginalConsent && original.captureGroupText) || groupApproval == consentTarget else { throw MemoryServiceDraftError.groupConsentRequired }
        return .setPreferences(.init(connectionID: connectionID, autoRecall: autoRecall, captureConversation: captureConversation,
            captureGroupText: captureGroupText, unattendedCapture: false, maxCaptureDeliveriesPerTurn: maxCaptureDeliveriesPerTurn,
            recallBudget: recallBudget), botID: botID)
    }
}

enum MemoryServiceAction: String, CaseIterable {
    case retain, recall, inspect, deleteDocument = "delete_document", clear
    case operationStatus = "operation_status", cancelOperation = "cancel_operation", reflect
    case bankProfile = "bank_profile", bankConfig = "bank_config", directives, mentalModels = "mental_models"
    case mentalModelHistory = "mental_model_history", observations, memoryEdit = "memory_edit"
    case memoryInvalidate = "memory_invalidate", memoryRestore = "memory_restore", documents, sessions, resources, tasks
    fileprivate static let basic: Set<Self> = [.retain, .recall, .inspect, .deleteDocument, .clear, .operationStatus, .cancelOperation, .reflect]
}
struct MemoryServiceCapabilities: Decodable {
    private var enabled: Set<MemoryServiceAction> = []
    private var advancedActions: [MemoryServiceAction: Set<String>] = [:]
    private(set) var idempotentRetain = false
    private(set) var writeFence = false
    init() {}
    private struct Key: CodingKey {
        let stringValue: String
        var intValue: Int? { nil }
        init(_ value: String) { stringValue = value }
        init?(stringValue: String) { self.stringValue = stringValue }
        init?(intValue: Int) { return nil }
    }
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: Key.self)
        idempotentRetain = try values.decodeIfPresent(Bool.self, forKey: Key("idempotent_retain")) ?? false
        writeFence = try values.decodeIfPresent(Bool.self, forKey: Key("write_fence")) ?? false
        for action in MemoryServiceAction.basic {
            if try values.decodeIfPresent(Bool.self, forKey: Key(action.rawValue)) == true { enabled.insert(action) }
        }
        let negotiated = try values.decodeIfPresent([String: [String]].self, forKey: Key("advanced_actions")) ?? [:]
        for name in try values.decodeIfPresent([String].self, forKey: Key("advanced")) ?? [] {
            if let action = MemoryServiceAction(rawValue: name), !MemoryServiceAction.basic.contains(action) {
                let verbs = Set(negotiated[name] ?? [])
                advancedActions[action] = verbs
                if verbs.contains(where: { action != .documents || $0 != "delete" }) { enabled.insert(action) }
            }
        }
    }
    func supports(_ action: MemoryServiceAction) -> Bool { enabled.contains(action) }
    func supports(_ feature: MemoryServiceAction, verb: String) -> Bool {
        enabled.contains(feature) && (feature != .documents || verb != "delete") && advancedActions[feature]?.contains(verb) == true
    }
}
enum MemoryServiceOperationState: String, Decodable {
    case queued, submitted, processing, completed, failed, deliveryUnknown = "delivery_unknown"
}

struct MemoryServiceRevision: Codable, Equatable { let counter: UInt64, deviceID: String
    private enum CodingKeys: String, CodingKey { case counter, deviceID = "device_id" }
}
struct MemoryServiceConnectionView: Decodable {
    enum Availability: String, Decodable { case supported, blocked }
    let id: String, revision: MemoryServiceRevision, backend: MemoryServiceBackend, name: String, hasSecret: Bool, embeddingProfile: String?
    let availability: Availability, reason: String?
    private enum CodingKeys: String, CodingKey { case id, revision, backend, name, availability, reason, hasSecret = "has_secret", embeddingProfile = "embedding_profile" }
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        id = try values.decode(String.self, forKey: .id); revision = try values.decode(MemoryServiceRevision.self, forKey: .revision)
        backend = try values.decode(MemoryServiceBackend.self, forKey: .backend); name = try values.decode(String.self, forKey: .name)
        hasSecret = try values.decode(Bool.self, forKey: .hasSecret); embeddingProfile = try values.decodeIfPresent(String.self, forKey: .embeddingProfile)
        availability = try values.decode(Availability.self, forKey: .availability)
        reason = try values.decode(String?.self, forKey: .reason)
    }
}
struct MemoryServiceEmbeddingView: Decodable {
    let id: String, revision: MemoryServiceRevision, model: String, modelRevision: String, dimensions: UInt32, hasSecret: Bool
    private enum CodingKeys: String, CodingKey { case id, revision, model, modelRevision = "model_revision", dimensions, hasSecret = "has_secret" }
}
struct MemoryServicePreferencesView: Decodable {
    let connectionID: String?, autoRecall: Bool, captureConversation: Bool, captureGroupText: Bool, unattendedCapture: Bool
    let maxCaptureDeliveriesPerTurn: Int, recallBudget: MemoryServiceRecallBudget, consentRevision: MemoryServiceRevision, deletionEpoch: UInt64
    private enum CodingKeys: String, CodingKey {
        case connectionID = "connection_id", autoRecall = "auto_recall", captureConversation = "capture_conversation", captureGroupText = "capture_group_text"
        case unattendedCapture = "unattended_capture", maxCaptureDeliveriesPerTurn = "max_capture_deliveries_per_turn", recallBudget = "recall_budget"
        case consentRevision = "consent_revision", deletionEpoch = "deletion_epoch"
    }
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        // Missing is malformed, not a request to disconnect the bot.
        connectionID = try values.decode(String?.self, forKey: .connectionID)
        autoRecall = try values.decode(Bool.self, forKey: .autoRecall)
        captureConversation = try values.decode(Bool.self, forKey: .captureConversation)
        captureGroupText = try values.decode(Bool.self, forKey: .captureGroupText)
        unattendedCapture = try values.decode(Bool.self, forKey: .unattendedCapture)
        maxCaptureDeliveriesPerTurn = try values.decode(Int.self, forKey: .maxCaptureDeliveriesPerTurn)
        recallBudget = try values.decode(MemoryServiceRecallBudget.self, forKey: .recallBudget)
        consentRevision = try values.decode(MemoryServiceRevision.self, forKey: .consentRevision)
        deletionEpoch = try values.decode(UInt64.self, forKey: .deletionEpoch)
        try recallBudget.validate()
        guard (0...4).contains(maxCaptureDeliveriesPerTurn) else { throw MemoryServiceDraftError.invalidCaptureCap }
    }
    var editable: MemoryServicePreferences {
        .init(connectionID: connectionID, autoRecall: autoRecall, captureConversation: captureConversation, captureGroupText: captureGroupText,
              unattendedCapture: unattendedCapture, maxCaptureDeliveriesPerTurn: maxCaptureDeliveriesPerTurn, recallBudget: recallBudget)
    }
}
struct MemoryServiceConnections: Decodable {
    let schemaVersion: UInt32, connections: [MemoryServiceConnectionView], embeddings: [MemoryServiceEmbeddingView], bots: [Bot]
    struct Bot: Decodable {
        let botID: String, preferences: MemoryServicePreferencesView
        private enum CodingKeys: String, CodingKey { case botID = "bot_id", preferences }
    }
    private enum Keys: String, CodingKey { case schemaVersion = "schema_version", connections, embeddings, bots }
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: Keys.self)
        schemaVersion = try values.decode(UInt32.self, forKey: .schemaVersion)
        guard schemaVersion == 1 else { throw DecodingError.dataCorruptedError(forKey: Keys.schemaVersion, in: values, debugDescription: "unsupported_memory_schema") }
        connections = try values.decode([MemoryServiceConnectionView].self, forKey: .connections)
        embeddings = try values.decode([MemoryServiceEmbeddingView].self, forKey: .embeddings)
        bots = try values.decode([Bot].self, forKey: .bots)
    }
}
struct MemoryServiceHealth: Decodable {
    enum Status: String, Decodable { case ready, degraded, setupRequired = "setup_required" }
    let status: Status, capabilities: MemoryServiceCapabilities, deletionPending: Bool
    let reason: String?
    private enum CodingKeys: String, CodingKey { case status, capabilities, reason, deletionPending = "deletion_pending" }
}
struct MemoryServiceOperation: Decodable {
    let id: String, documentID: String, state: MemoryServiceOperationState, operationID: String?, errorCode: String?
    private enum CodingKeys: String, CodingKey { case id, documentID = "document_id", state, operationID = "operation_id", errorCode = "error_code" }
}
struct MemoryServiceOperations: Decodable {
    let operations: [MemoryServiceOperation], deletion: Deletion?
    struct Deletion: Decodable {
        let pending: Bool, operationID: String?, deletionEpoch: UInt64
        private enum CodingKeys: String, CodingKey { case pending, operationID = "operation_id", deletionEpoch = "deletion_epoch" }
    }
}

enum MemoryServiceRequest {
    case listConnections, setConnection(MemoryServiceConnectionEdit), disconnectConnection(id: String)
    case getPreferences(botID: String), setPreferences(MemoryServicePreferences, botID: String)
    case health(botID: String), operations(botID: String)
    var method: String {
        switch self {
        case .listConnections: return "memory.connections.list"
        case .setConnection: return "memory.connections.set"
        case .disconnectConnection: return "memory.connections.disconnect"
        case .getPreferences: return "memory.preferences.get"
        case .setPreferences: return "memory.preferences.set"
        case .health: return "memory.service.health"
        case .operations: return "memory.operations.list"
        }
    }
    private struct BotParameters: Encodable {
        let botID: String
        enum CodingKeys: String, CodingKey { case botID = "bot_id" }
    }
    private struct IDParameters: Encodable { let id: String }
    private struct PreferenceParameters: Encodable {
        let preferences: MemoryServicePreferences, botID: String
        func encode(to encoder: Encoder) throws {
            try preferences.encode(to: encoder)
            var values = encoder.container(keyedBy: BotParameters.CodingKeys.self)
            try values.encode(botID, forKey: .botID)
        }
    }
    func parameters() throws -> Data {
        let encoder = JSONEncoder()
        switch self {
        case .listConnections: return Data("{}".utf8)
        case let .setConnection(edit): return try encoder.encode(edit)
        case let .disconnectConnection(id): return try encoder.encode(IDParameters(id: id))
        case let .getPreferences(botID), let .health(botID), let .operations(botID): return try encoder.encode(BotParameters(botID: botID))
        case let .setPreferences(preferences, botID): return try encoder.encode(PreferenceParameters(preferences: preferences, botID: botID))
        }
    }
}

// Uses the app's existing transport when ownership releases; never opens another socket.
// Typed reads expose masked fields only; config-write replies remain raw until frozen.
struct MemoryServiceAPI {
    let transport: (String, Data) async throws -> Data
    func send(_ request: MemoryServiceRequest) async throws -> Data {
        try await transport(request.method, request.parameters())
    }
    func savePreferences(_ draft: MemoryServicePreferencesDraft) async throws -> Data {
        try await send(draft.request())
    }
    func listConnections() async throws -> MemoryServiceConnections {
        try JSONDecoder().decode(MemoryServiceConnections.self, from: await send(.listConnections))
    }
    func getPreferences(botID: String) async throws -> MemoryServicePreferencesView {
        try JSONDecoder().decode(MemoryServicePreferencesView.self, from: await send(.getPreferences(botID: botID)))
    }
    func health(botID: String) async throws -> MemoryServiceHealth {
        try JSONDecoder().decode(MemoryServiceHealth.self, from: await send(.health(botID: botID)))
    }
    func operations(botID: String) async throws -> MemoryServiceOperations {
        try JSONDecoder().decode(MemoryServiceOperations.self, from: await send(.operations(botID: botID)))
    }
}
