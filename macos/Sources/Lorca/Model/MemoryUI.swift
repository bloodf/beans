import Foundation

enum MemoryUIError: String, Error, LocalizedError {
    case unavailable = "memory_unavailable", unsupported = "unsupported_memory_operation", invalidInput = "invalid_memory_input"
    case staleTarget = "memory_target_changed", confirmationRequired = "confirmation_required", invalidReply = "invalid_memory_reply"
    case lanceCloudTransportUnavailable = "lancedb_cloud_transport_unavailable"
    var errorDescription: String? {
        self == .lanceCloudTransportUnavailable ? rawValue : rawValue.replacingOccurrences(of: "_", with: " ")
    }
}

enum MemoryUIRPC {
    static let allowed: Set<String> = [
        "memory.connections.list", "memory.connections.set", "memory.connections.disconnect",
        "memory.embeddings.set", "memory.embeddings.remove", "memory.preferences.get", "memory.preferences.set",
        "memory.service.health", "memory.service.recall", "memory.service.retain", "memory.service.inspect",
        "memory.service.reflect", "memory.service.advanced", "memory.service.delete", "memory.operations.list",
        "memory.operations.retry", "memory.operations.status", "memory.operations.cancel",
        "memory.pgvector.initialize.preview", "memory.pgvector.initialize.apply",
        "memory.lance.binding.preview", "memory.lance.binding.apply", "memory.lance.export.preview",
        "memory.lance.export.apply", "memory.lance.import.preview", "memory.lance.import.apply",
        "memory.embeddings.local.preview", "memory.embeddings.local.apply", "memory.embeddings.local.status",
    ]
}

struct MemoryEmbeddingProfile: Encodable {
    enum Mode: String, Encodable { case api, localCPU = "local_cpu" }
    struct Local: Encodable {
        struct Tensors: Encodable { let input_ids: String, attention_mask: String, token_type_ids: String?, output: String }
        let model_sha256: String, tokenizer_sha256: String, max_tokens: Int, pooling: String, tensors: Tensors
        let add_special_tokens: Bool, pad_id: UInt32, pad_type_id: UInt32, pad_token: String
    }
    let model: String, revision: String, dimensions: UInt32, normalization: String, distance: String
    let document_prefix: String, query_prefix: String, endpoint: String?, mode: Mode, local: Local?
    func validate() throws {
        guard !model.isEmpty, !revision.isEmpty, (1...65536).contains(dimensions),
              ["none", "l2"].contains(normalization), ["cosine", "dot", "euclidean"].contains(distance),
              document_prefix.utf8.count <= 4096, query_prefix.utf8.count <= 4096 else { throw MemoryUIError.invalidInput }
        if mode == .api {
            guard local == nil, let endpoint, endpoint.hasSuffix("/embeddings") else { throw MemoryUIError.invalidInput }
            try MemoryUIValidation.endpoint(endpoint, schemes: ["https", "http"])
        } else {
            guard endpoint == nil, let local, (1...8192).contains(local.max_tokens),
                  ["mean", "cls", "pooled"].contains(local.pooling), local.pad_token.utf8.count <= 256 else { throw MemoryUIError.invalidInput }
            try MemoryUIValidation.hash(local.model_sha256); try MemoryUIValidation.hash(local.tokenizer_sha256)
            let names = [local.tensors.input_ids, local.tensors.attention_mask] + (local.tensors.token_type_ids.map { [$0] } ?? [])
            guard Set(names).count == names.count, (names + [local.tensors.output]).allSatisfy({ !$0.isEmpty && $0.utf8.count <= 256 && !$0.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains) }) else { throw MemoryUIError.invalidInput }
        }
    }
}

enum MemoryUIValidation {
    static func endpoint(_ value: String, schemes: Set<String>) throws {
        guard let url = URLComponents(string: value), schemes.contains(url.scheme ?? ""),
              let host = url.host, !host.isEmpty, url.user == nil, url.password == nil,
              url.query == nil, url.fragment == nil else { throw MemoryUIError.invalidInput }
    }
    static func hash(_ value: String) throws {
        guard value.count == 64, value.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }) else { throw MemoryUIError.invalidInput }
    }
    static func identifier(_ value: String) throws {
        guard !value.isEmpty, value.utf8.count <= 256,
              value.utf8.allSatisfy({ (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0) || $0 == 45 || $0 == 95 }) else { throw MemoryUIError.invalidInput }
    }
    static func resourcePath(_ value: String, allowRoot: Bool) throws {
        guard (allowRoot && value == "resources") || value.hasPrefix("resources/"),
              !value.contains("%"), !value.contains("\\"), !value.contains(":"),
              value.split(separator: "/", omittingEmptySubsequences: false).allSatisfy({ !$0.isEmpty && $0 != "." && $0 != ".." }) else { throw MemoryUIError.invalidInput }
    }
}

struct MemoryAdvancedCommand {
    let feature: MemoryServiceAction, action: String, body: [String: Any]
}

extension MemoryServiceAPI {
    func request(_ method: String, _ parameters: [String: Any]) async throws -> Data {
        guard MemoryUIRPC.allowed.contains(method), JSONSerialization.isValidJSONObject(parameters) else { throw MemoryUIError.unsupported }
        return try await transport(method, JSONSerialization.data(withJSONObject: parameters))
    }
    func saveEmbedding(id: String, profile: MemoryEmbeddingProfile, secret: MemoryServiceSecretPatch) async throws {
        try profile.validate()
        struct Parameters: Encodable { let id: String, profile: MemoryEmbeddingProfile, secret: MemoryServiceSecretPatch }
        try await saved(transport("memory.embeddings.set", JSONEncoder().encode(Parameters(id: id, profile: profile, secret: secret))))
    }
    func saved(_ response: Data) throws {
        struct Reply: Decodable { let saved: Bool }
        guard try JSONDecoder().decode(Reply.self, from: response).saved else { throw MemoryUIError.invalidReply }
    }
    func advanced(botID: String, command: MemoryAdvancedCommand) async throws -> Data {
        try await request("memory.service.advanced", ["bot_id": botID, "feature": command.feature.rawValue, "action": command.action, "body": command.body])
    }
}
