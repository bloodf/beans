import Foundation

// Exact concrete setup contract; these helpers do not execute without the injected UI transport.
enum MemorySetupError: String, Error {
    case invalidReply = "invalid_setup_reply", invalidAssetPlan = "invalid_asset_plan", invalidAssetSource = "invalid_asset_source"
    case confirmationRequired = "confirmation_required", approvalNotFound = "approval_not_found", approvalExpired = "approval_expired"
    case approvalStale = "approval_stale", approvalActionMismatch = "approval_action_mismatch"
}
enum MemoryAssetKind: String, Codable { case runtime, model, tokenizer }
enum MemoryAssetSource: Codable {
    case supplied(path: String), download(url: String)
    private enum Keys: String, CodingKey { case kind, path, url }
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: Keys.self)
        switch try values.decode(String.self, forKey: .kind) {
        case "supplied": self = .supplied(path: try values.decode(String.self, forKey: .path))
        case "download": self = .download(url: try values.decode(String.self, forKey: .url))
        default: throw MemorySetupError.invalidAssetSource
        }
    }
    func encode(to encoder: Encoder) throws {
        var values = encoder.container(keyedBy: Keys.self)
        switch self {
        case let .supplied(path): try values.encode("supplied", forKey: .kind); try values.encode(path, forKey: .path)
        case let .download(url): try values.encode("download", forKey: .kind); try values.encode(url, forKey: .url)
        }
    }
}
struct MemoryAssetRequest: Codable {
    let kind: MemoryAssetKind, source: MemoryAssetSource, license: String, bytes: UInt64, sha256: String
    func validate() throws {
        guard bytes > 0, !license.isEmpty, sha256.count == 64,
              sha256.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }) else { throw MemorySetupError.invalidAssetPlan }
        switch source {
        case let .supplied(path): guard !path.isEmpty else { throw MemorySetupError.invalidAssetSource }
        case let .download(value):
            guard let url = URLComponents(string: value), url.scheme == "https", url.host != nil,
                  url.user == nil, url.password == nil, url.fragment == nil else { throw MemorySetupError.invalidAssetSource }
        }
    }
}
struct MemoryAssetPlan: Codable {
    let assets: [MemoryAssetRequest]
    func validate() throws {
        guard assets.count == 3, Set(assets.map(\.kind)) == Set([.runtime, .model, .tokenizer]) else { throw MemorySetupError.invalidAssetPlan }
        for asset in assets { try asset.validate() }
    }
}
struct MemoryLocalAssetTarget: Encodable, Equatable {
    let runnerID: String, profileID: String, profileRevision: MemoryServiceRevision
    private enum CodingKeys: String, CodingKey { case runnerID = "runner_id", profileID = "profile_id", profileRevision = "profile_revision" }
}
struct MemoryBotSetupTarget: Equatable { let runnerID: String, botID: String, connectionRevision: MemoryServiceRevision }
struct MemoryLocalAssetPreview: Decodable {
    let previewToken: String, previewDigest: String, expiresInSeconds: UInt64, runnerID: String, profileID: String
    let profileRevision: MemoryServiceRevision, totalBytes: UInt64, assets: [MemoryAssetRequest]
    private enum CodingKeys: String, CodingKey {
        case previewToken = "preview_token", previewDigest = "preview_digest", expiresInSeconds = "expires_in_seconds"
        case runnerID = "runner_id", profileID = "profile_id", profileRevision = "profile_revision", totalBytes = "total_bytes", assets
    }
    var target: MemoryLocalAssetTarget { .init(runnerID: runnerID, profileID: profileID, profileRevision: profileRevision) }
    func validate() throws {
        try MemoryAssetPlan(assets: assets).validate()
        var total: UInt64 = 0
        for asset in assets {
            let next = total.addingReportingOverflow(asset.bytes)
            guard !next.overflow else { throw MemorySetupError.invalidReply }
            total = next.partialValue
        }
        guard expiresInSeconds == 600, !previewToken.isEmpty, previewDigest.count == 64,
              previewDigest.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }), total == totalBytes else { throw MemorySetupError.invalidReply }
    }
}
enum MemoryBotSetupAction: String, Codable {
    case pgvector = "memory.pgvector.initialize.preview", lanceBinding = "memory.lance.binding.preview"
    case lanceExport = "memory.lance.export.preview", lanceImport = "memory.lance.import.preview"
    var applyMethod: String {
        switch self {
        case .pgvector: return "memory.pgvector.initialize.apply"
        case .lanceBinding: return "memory.lance.binding.apply"
        case .lanceExport: return "memory.lance.export.apply"
        case .lanceImport: return "memory.lance.import.apply"
        }
    }
}
struct MemoryPgvectorDetails: Decodable {
    struct Target: Decodable {
        let database: String, databaseOID: UInt32, role: String, serverAddress: String?, serverPort: Int32?, serverVersion: String, sessionPID: Int32
        private enum CodingKeys: String, CodingKey {
            case database, databaseOID = "database_oid", role, serverAddress = "server_address", serverPort = "server_port", serverVersion = "server_version", sessionPID = "session_pid"
        }
    }
    struct Readiness: Decodable {
        let ready: Bool, extensionVersion: String?, extensionSchema: String?
        let schemaExists: Bool, canCreateSchema: Bool, canCreateTables: Bool, canInstallExtension: Bool
        private enum CodingKeys: String, CodingKey {
            case ready, extensionVersion = "extension_version", extensionSchema = "extension_schema"
            case schemaExists = "schema_exists", canCreateSchema = "can_create_schema"
            case canCreateTables = "can_create_tables", canInstallExtension = "can_install_extension"
        }
    }
    let target: Target, schema: String, sql: String
    let readiness: Readiness
}
struct MemoryLanceBindingDetails: Decodable { let directory: String, create: Bool, table: String, namespace: String }
struct MemoryVectorSpace: Decodable {
    enum Distance: String, Decodable { case cosine, dot, euclidean }
    let fingerprint: String, dimensions: UInt64, distance: Distance, generation: UInt64
}
struct MemoryLanceTransferDetails: Decodable { let path: String, namespace: String, space: MemoryVectorSpace, documents: UInt64, sha256: String }
struct MemoryBotSetupPreview<Details: Decodable>: Decodable {
    let token: String, expiresAt: Double, action: MemoryBotSetupAction, runnerID: String, botID: String
    let connectionRevision: MemoryServiceRevision, details: Details
    private enum Keys: String, CodingKey {
        case token, expiresAt = "expires_at", action, runnerID = "runner_id", botID = "bot_id", connectionRevision = "connection_revision", details
        case profileID = "profile_id", profileRevision = "profile_revision"
    }
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: Keys.self)
        guard try values.decode(String?.self, forKey: .profileID) == nil,
              try values.decode(MemoryServiceRevision?.self, forKey: .profileRevision) == nil else { throw MemorySetupError.invalidReply }
        token = try values.decode(String.self, forKey: .token); expiresAt = try values.decode(Double.self, forKey: .expiresAt)
        action = try values.decode(MemoryBotSetupAction.self, forKey: .action); runnerID = try values.decode(String.self, forKey: .runnerID)
        botID = try values.decode(String.self, forKey: .botID); connectionRevision = try values.decode(MemoryServiceRevision.self, forKey: .connectionRevision)
        details = try values.decode(Details.self, forKey: .details)
        guard !token.isEmpty, !runnerID.isEmpty, !botID.isEmpty, expiresAt.isFinite, expiresAt > 0 else { throw MemorySetupError.invalidReply }
    }
    var target: MemoryBotSetupTarget { .init(runnerID: runnerID, botID: botID, connectionRevision: connectionRevision) }
}

enum MemorySetupRequest {
    case localPreview(target: MemoryLocalAssetTarget, plan: MemoryAssetPlan), localApply(runnerID: String, token: String, digest: String)
    case localStatus(runnerID: String, profileID: String), pgvectorPreview(botID: String)
    case lanceBindingPreview(botID: String, directory: String, create: Bool), lanceTransferPreview(action: MemoryBotSetupAction, botID: String, path: String)
    case botApply(action: MemoryBotSetupAction, botID: String, token: String)
    var method: String {
        switch self {
        case .localPreview: return "memory.embeddings.local.preview"
        case .localApply: return "memory.embeddings.local.apply"
        case .localStatus: return "memory.embeddings.local.status"
        case .pgvectorPreview: return MemoryBotSetupAction.pgvector.rawValue
        case .lanceBindingPreview: return MemoryBotSetupAction.lanceBinding.rawValue
        case let .lanceTransferPreview(action, _, _): return action.rawValue
        case let .botApply(action, _, _): return action.applyMethod
        }
    }
    private struct LocalPreview: Encodable {
        let runnerID: String, profileID: String, profileRevision: MemoryServiceRevision, plan: MemoryAssetPlan
        enum CodingKeys: String, CodingKey { case runnerID = "runner_id", profileID = "profile_id", profileRevision = "profile_revision", plan }
    }
    private struct LocalApply: Encodable {
        let runnerID: String, previewToken: String, previewDigest: String, confirm = true
        enum CodingKeys: String, CodingKey { case runnerID = "runner_id", previewToken = "preview_token", previewDigest = "preview_digest", confirm }
    }
    private struct LocalStatus: Encodable {
        let runnerID: String, profileID: String
        enum CodingKeys: String, CodingKey { case runnerID = "runner_id", profileID = "profile_id" }
    }
    private struct Bot: Encodable { let botID: String; enum CodingKeys: String, CodingKey { case botID = "bot_id" } }
    private struct Binding: Encodable { let botID: String, directory: String, create: Bool; enum CodingKeys: String, CodingKey { case botID = "bot_id", directory, create } }
    private struct Transfer: Encodable { let botID: String, path: String; enum CodingKeys: String, CodingKey { case botID = "bot_id", path } }
    private struct BotApply: Encodable { let botID: String, token: String, confirm = true; enum CodingKeys: String, CodingKey { case botID = "bot_id", token, confirm } }
    func parameters() throws -> Data {
        let encoder = JSONEncoder()
        switch self {
        case let .localPreview(target, plan):
            try plan.validate()
            return try encoder.encode(LocalPreview(runnerID: target.runnerID, profileID: target.profileID, profileRevision: target.profileRevision, plan: plan))
        case let .localApply(runnerID, token, digest): return try encoder.encode(LocalApply(runnerID: runnerID, previewToken: token, previewDigest: digest))
        case let .localStatus(runnerID, profileID): return try encoder.encode(LocalStatus(runnerID: runnerID, profileID: profileID))
        case let .pgvectorPreview(botID): return try encoder.encode(Bot(botID: botID))
        case let .lanceBindingPreview(botID, directory, create): return try encoder.encode(Binding(botID: botID, directory: directory, create: create))
        case let .lanceTransferPreview(action, botID, path):
            guard action == .lanceExport || action == .lanceImport else { throw MemorySetupError.approvalActionMismatch }
            return try encoder.encode(Transfer(botID: botID, path: path))
        case let .botApply(_, botID, token): return try encoder.encode(BotApply(botID: botID, token: token))
        }
    }
}
final class MemoryLocalAssetApproval {
    let preview: MemoryLocalAssetPreview
    private let deadline: Date
    private var used = false
    init(preview: MemoryLocalAssetPreview, receivedAt: Date = Date()) { self.preview = preview; deadline = receivedAt.addingTimeInterval(Double(preview.expiresInSeconds)) }
    func applyRequest(confirm: Bool, current: MemoryLocalAssetTarget, now: Date = Date()) throws -> MemorySetupRequest {
        guard confirm else { throw MemorySetupError.confirmationRequired }
        guard !used else { throw MemorySetupError.approvalNotFound }
        guard now < deadline else { throw MemorySetupError.approvalExpired }
        guard current == preview.target else { throw MemorySetupError.approvalStale }
        try preview.validate()
        used = true
        return .localApply(runnerID: preview.runnerID, token: preview.previewToken, digest: preview.previewDigest)
    }
}
final class MemoryBotSetupApproval<Details: Decodable> {
    let preview: MemoryBotSetupPreview<Details>
    private var used = false
    init(preview: MemoryBotSetupPreview<Details>, expected: MemoryBotSetupAction) throws {
        guard preview.action == expected else { throw MemorySetupError.approvalActionMismatch }
        self.preview = preview
    }
    func applyRequest(confirm: Bool, current: MemoryBotSetupTarget, now: Date = Date()) throws -> MemorySetupRequest {
        guard confirm else { throw MemorySetupError.confirmationRequired }
        guard !used else { throw MemorySetupError.approvalNotFound }
        guard now.timeIntervalSince1970 < Double(preview.expiresAt) else { throw MemorySetupError.approvalExpired }
        guard current == preview.target else { throw MemorySetupError.approvalStale }
        used = true
        return .botApply(action: preview.action, botID: preview.botID, token: preview.token)
    }
}
enum MemoryLocalEmbeddingStatus: String, Decodable { case ready, setupRequired = "setup_required", runtimeUnavailable = "runtime_unavailable" }
struct MemoryLocalSetupResult: Decodable { let installed: Bool, status: MemoryLocalEmbeddingStatus }
struct MemoryLocalStatusResult: Decodable { let status: MemoryLocalEmbeddingStatus }
struct MemoryPgvectorSetupResult: Decodable { let initialized: Bool }
struct MemoryLanceBindingResult: Decodable { let status: MemoryLocalEmbeddingStatus }
struct MemoryLanceExportResult: Decodable { let exported: Bool, bytes: UInt64 }
struct MemoryLanceImportResult: Decodable { let imported: Bool }
enum MemoryBotSetupResult { case initialized, bindingReady, exported(bytes: UInt64), imported }
struct MemorySetupAPI {
    let transport: (String, Data) async throws -> Data
    func send(_ request: MemorySetupRequest) async throws -> Data { try await transport(request.method, request.parameters()) }
    func previewLocal(target: MemoryLocalAssetTarget, plan: MemoryAssetPlan) async throws -> MemoryLocalAssetPreview {
        let reply = try JSONDecoder().decode(MemoryLocalAssetPreview.self, from: await send(.localPreview(target: target, plan: plan)))
        try reply.validate()
        guard reply.target == target else { throw MemorySetupError.approvalStale }
        return reply
    }
    func previewBot<Details: Decodable>(_ request: MemorySetupRequest, as: Details.Type) async throws -> MemoryBotSetupPreview<Details> {
        let reply = try JSONDecoder().decode(MemoryBotSetupPreview<Details>.self, from: await send(request))
        guard reply.action.rawValue == request.method else { throw MemorySetupError.approvalActionMismatch }
        let expectedBot: String
        switch request {
        case let .pgvectorPreview(botID), let .lanceBindingPreview(botID, _, _), let .lanceTransferPreview(_, botID, _): expectedBot = botID
        default: throw MemorySetupError.approvalActionMismatch
        }
        guard reply.botID == expectedBot else { throw MemorySetupError.approvalStale }
        return reply
    }
    func localStatus(runnerID: String, profileID: String) async throws -> MemoryLocalStatusResult {
        try JSONDecoder().decode(MemoryLocalStatusResult.self, from: await send(.localStatus(runnerID: runnerID, profileID: profileID)))
    }
    func applyLocal(_ approval: MemoryLocalAssetApproval, confirm: Bool, current: MemoryLocalAssetTarget, now: Date = Date()) async throws -> MemoryLocalSetupResult {
        let request = try approval.applyRequest(confirm: confirm, current: current, now: now)
        let reply = try JSONDecoder().decode(MemoryLocalSetupResult.self, from: await send(request))
        guard reply.installed, reply.status == .ready || reply.status == .runtimeUnavailable else { throw MemorySetupError.invalidReply }
        return reply
    }
    func applyBot<Details: Decodable>(_ approval: MemoryBotSetupApproval<Details>, confirm: Bool, current: MemoryBotSetupTarget, now: Date = Date()) async throws -> MemoryBotSetupResult {
        let request = try approval.applyRequest(confirm: confirm, current: current, now: now)
        let data = try await send(request)
        switch approval.preview.action {
        case .pgvector:
            guard try JSONDecoder().decode(MemoryPgvectorSetupResult.self, from: data).initialized else { throw MemorySetupError.invalidReply }
            return .initialized
        case .lanceBinding:
            guard try JSONDecoder().decode(MemoryLanceBindingResult.self, from: data).status == .ready else { throw MemorySetupError.invalidReply }
            return .bindingReady
        case .lanceExport:
            let result = try JSONDecoder().decode(MemoryLanceExportResult.self, from: data)
            guard result.exported, result.bytes <= 8 * 1024 * 1024 else { throw MemorySetupError.invalidReply }
            return .exported(bytes: result.bytes)
        case .lanceImport:
            guard try JSONDecoder().decode(MemoryLanceImportResult.self, from: data).imported else { throw MemorySetupError.invalidReply }
            return .imported
        }
    }
}
