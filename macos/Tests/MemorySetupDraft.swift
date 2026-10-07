import Foundation

private func check(_ value: Bool, _ message: String) { if !value { fatalError(message) } }
private func rejects(_ code: MemorySetupError, _ body: () throws -> Void) {
    do { try body(); fatalError("expected \(code)") }
    catch let error as MemorySetupError { check(error == code, "wrong setup refusal") }
    catch { fatalError("unexpected setup error") }
}
private func object(_ request: MemorySetupRequest) throws -> [String: Any] { try JSONSerialization.jsonObject(with: request.parameters()) as! [String: Any] }

@main struct MemorySetupDraftTests {
    static func main() async throws {
        let revision = MemoryServiceRevision(counter: 3, deviceID: "fixture")
        let assets: [MemoryAssetRequest] = [.runtime, .model, .tokenizer].map {
            .init(kind: $0, source: .supplied(path: "/fixture/\($0.rawValue)"), license: "Fixture license", bytes: 10, sha256: String(repeating: "a", count: 64))
        }
        let target = MemoryLocalAssetTarget(runnerID: "runner-a", profileID: "local", profileRevision: revision)
        let previewRequest = MemorySetupRequest.localPreview(target: target, plan: .init(assets: assets))
        check(previewRequest.method == "memory.embeddings.local.preview", "invented preview method")
        let params = try object(previewRequest)
        check(params["runner_id"] as? String == "runner-a" && params["profile_revision"] != nil && params["plan"] != nil, "local scope lost")
        rejects(.invalidAssetPlan) { _ = try MemorySetupRequest.localPreview(target: target, plan: .init(assets: Array(assets.dropFirst()))).parameters() }
        let localData = try JSONSerialization.data(withJSONObject: ["preview_token": "opaque", "preview_digest": String(repeating: "b", count: 64), "expires_in_seconds": 600, "runner_id": "runner-a", "profile_id": "local", "profile_revision": ["counter": 3, "device_id": "fixture"], "total_bytes": 30, "assets": JSONSerialization.jsonObject(with: JSONEncoder().encode(assets))])
        let local = try JSONDecoder().decode(MemoryLocalAssetPreview.self, from: localData)
        let approval = MemoryLocalAssetApproval(preview: local, receivedAt: Date(timeIntervalSince1970: 0))
        rejects(.confirmationRequired) { _ = try approval.applyRequest(confirm: false, current: target, now: Date(timeIntervalSince1970: 1)) }
        rejects(.approvalStale) { _ = try approval.applyRequest(confirm: true, current: .init(runnerID: "other", profileID: "local", profileRevision: revision), now: Date(timeIntervalSince1970: 1)) }
        let apply = try approval.applyRequest(confirm: true, current: target, now: Date(timeIntervalSince1970: 1))
        check(apply.method == "memory.embeddings.local.apply", "invented apply alias")
        check(Set(try object(apply).keys) == Set(["runner_id", "preview_token", "preview_digest", "confirm"]), "apply accepts override")
        rejects(.approvalNotFound) { _ = try approval.applyRequest(confirm: true, current: target, now: Date(timeIntervalSince1970: 2)) }
        rejects(.approvalExpired) { _ = try MemoryLocalAssetApproval(preview: local, receivedAt: Date(timeIntervalSince1970: 0)).applyRequest(confirm: true, current: target, now: Date(timeIntervalSince1970: 600)) }
        let botJSON = Data("{\"token\":\"bot-token\",\"expires_at\":400.5,\"action\":\"memory.pgvector.initialize.preview\",\"runner_id\":\"runner-a\",\"bot_id\":\"bot-a\",\"profile_id\":null,\"connection_revision\":{\"counter\":3,\"device_id\":\"fixture\"},\"profile_revision\":null,\"details\":{\"target\":{\"database\":\"memory\",\"database_oid\":42,\"role\":\"user\",\"server_address\":null,\"server_port\":5432,\"server_version\":\"17\",\"session_pid\":22},\"schema\":\"beans\",\"sql\":\"SELECT 'fixture';\",\"readiness\":{\"ready\":false,\"extension_version\":null,\"extension_schema\":null,\"schema_exists\":false,\"can_create_schema\":true,\"can_create_tables\":true,\"can_install_extension\":false}}}".utf8)
        let bot = try JSONDecoder().decode(MemoryBotSetupPreview<MemoryPgvectorDetails>.self, from: botJSON)
        check(!bot.details.readiness.ready && !bot.details.readiness.canInstallExtension && bot.details.readiness.canCreateTables, "actual schema permissions lost")
        let botApproval = try MemoryBotSetupApproval(preview: bot, expected: .pgvector)
        let botTarget = MemoryBotSetupTarget(runnerID: "runner-a", botID: "bot-a", connectionRevision: revision)
        rejects(.approvalActionMismatch) { _ = try MemoryBotSetupApproval(preview: bot, expected: .lanceBinding) }
        rejects(.approvalStale) { _ = try botApproval.applyRequest(confirm: true, current: .init(runnerID: "runner-a", botID: "other", connectionRevision: revision), now: Date(timeIntervalSince1970: 100)) }
        let botApply = try botApproval.applyRequest(confirm: true, current: botTarget, now: Date(timeIntervalSince1970: 100))
        let botApplyKeys = Set(try object(botApply).keys)
        check(botApply.method == "memory.pgvector.initialize.apply" && botApplyKeys == Set(["bot_id", "token", "confirm"]), "bot apply retargets SQL/path")
        var lanceJSON = try JSONSerialization.jsonObject(with: botJSON) as! [String: Any]
        lanceJSON["action"] = "memory.lance.binding.preview"
        lanceJSON["details"] = ["directory": "/fixture/db", "create": true, "table": "beans_memory_v1", "namespace": String(repeating: "c", count: 64)]
        let binding = try JSONDecoder().decode(MemoryBotSetupPreview<MemoryLanceBindingDetails>.self, from: JSONSerialization.data(withJSONObject: lanceJSON))
        check(binding.details.create && binding.details.directory == "/fixture/db", "binding preview target lost")
        let bindingApproval = try MemoryBotSetupApproval(preview: binding, expected: .lanceBinding)
        let bindingApply = try bindingApproval.applyRequest(confirm: true, current: botTarget, now: Date(timeIntervalSince1970: 100))
        check(Set(try object(bindingApply).keys) == Set(["bot_id", "token", "confirm"]), "binding apply overrides namespace/path")
        for action in [MemoryBotSetupAction.lanceExport, .lanceImport] {
            lanceJSON["action"] = action.rawValue
            lanceJSON["details"] = ["path": "/fixture/transfer.json", "namespace": String(repeating: "c", count: 64), "space": ["fingerprint": String(repeating: "d", count: 64), "dimensions": 3, "distance": "cosine", "generation": 1], "documents": 2, "sha256": String(repeating: "e", count: 64)]
            let transfer = try JSONDecoder().decode(MemoryBotSetupPreview<MemoryLanceTransferDetails>.self, from: JSONSerialization.data(withJSONObject: lanceJSON))
            check(transfer.details.space.fingerprint == String(repeating: "d", count: 64) && transfer.details.documents == 2, "transfer lost full space metadata")
            let transferApproval = try MemoryBotSetupApproval(preview: transfer, expected: action)
            check(try transferApproval.applyRequest(confirm: true, current: botTarget, now: Date(timeIntervalSince1970: 100)).method == action.applyMethod, "wrong transfer method")
        }
        var calls: [String] = []
        let api = MemorySetupAPI { method, _ in calls.append(method); throw MemorySetupError.invalidReply }
        let failureApproval = MemoryLocalAssetApproval(preview: local, receivedAt: Date(timeIntervalSince1970: 0))
        do { _ = try await api.applyLocal(failureApproval, confirm: true, current: target, now: Date(timeIntervalSince1970: 1)); fatalError("fake installed") }
        catch let error as MemorySetupError { check(error == .invalidReply, "wrong fixture error") }
        rejects(.approvalNotFound) { _ = try failureApproval.applyRequest(confirm: true, current: target, now: Date(timeIntervalSince1970: 2)) }
        check(calls == ["memory.embeddings.local.apply"], "retry sent twice")
        print("MemorySetupDraft: exact routes, scope, confirmation, expiry and failed-apply single-use checks passed")
    }
}
