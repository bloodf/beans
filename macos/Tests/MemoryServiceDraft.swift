import Foundation

private func check(_ condition: Bool, _ message: String) {
    if !condition { fatalError(message) }
}
private func rejects(_ code: MemoryServiceDraftError, _ body: () throws -> Void) {
    do { try body(); fatalError("expected \(code)") }
    catch let error as MemoryServiceDraftError { check(error == code, "wrong error: \(error)") }
    catch { fatalError("unexpected error: \(error)") }
}
private func object(_ request: MemoryServiceRequest) throws -> [String: Any] {
    try JSONSerialization.jsonObject(with: request.parameters()) as! [String: Any]
}

@main
struct MemoryServiceDraftTests {
    static func main() async throws {
        var draft = MemoryServicePreferencesDraft(botID: "bot-a")
        let defaults = try object(draft.request())
        check(defaults["connection_id"] is NSNull, "no connection must be explicit null")
        for key in ["auto_recall", "capture_conversation", "capture_group_text", "unattended_capture"] {
            check(defaults[key] as? Bool == false, "new draft enables \(key)")
        }
        draft.connectionID = "a"
        draft.autoRecall = true
        rejects(.plaintextConsentRequired) { _ = try draft.request() }
        draft.approveRemotePlaintext()
        check(try object(draft.request())["bot_id"] as? String == "bot-a", "wrong bot")
        draft.connectionID = "b"
        draft.connectionID = "a"
        rejects(.plaintextConsentRequired) { _ = try draft.request() }
        draft.captureConversation = true
        draft.captureGroupText = true
        draft.approveRemotePlaintext()
        rejects(.groupConsentRequired) { _ = try draft.request() }
        draft.approveGroupCapture()
        let approved = try object(draft.request())
        check(approved["capture_group_text"] as? Bool == true, "group consent lost")
        check(Set(approved.keys) == Set(["bot_id", "connection_id", "auto_recall", "capture_conversation", "capture_group_text", "unattended_capture", "max_capture_deliveries_per_turn", "recall_budget"]), "extra scope/backfill fields")
        draft.maxCaptureDeliveriesPerTurn = 2
        rejects(.plaintextConsentRequired) { _ = try draft.request() }
        draft.unattendedCapture = true
        rejects(.enforcedSpendingPolicyRequired) { _ = try draft.request() }
        draft.unattendedCapture = false
        draft.maxCaptureDeliveriesPerTurn = 5
        rejects(.invalidCaptureCap) { _ = try draft.request() }

        var bounded = MemoryServicePreferencesDraft(botID: "bot-b")
        bounded.recallBudget.timeoutMS = 5001
        rejects(.invalidBudget) { _ = try bounded.request() }
        bounded.recallBudget = .init(timeoutMS: 5000, maxBytes: 32768, maxResults: 20, maxContextChars: 8000)
        _ = try bounded.request()
        bounded.recallBudget.maxContextChars = 99
        rejects(.invalidBudget) { _ = try bounded.request() }

        var saved = MemoryServicePreferences()
        saved.connectionID = "a"; saved.captureConversation = true; saved.captureGroupText = true
        var editing = MemoryServicePreferencesDraft(botID: "bot-c", saved: saved)
        editing.recallBudget.maxResults = 4
        check(saved.recallBudget.maxResults == 8, "draft mutated saved budget")
        _ = try editing.request()
        editing.captureConversation = false; editing.captureGroupText = false
        check(try object(editing.request())["capture_conversation"] as? Bool == false, "cannot disable capture")

        let key = " exact key "
        let replacement = MemoryServiceSecretPatch.replace(key)
        check(!String(describing: replacement).contains(key), "description reveals replacement")
        check(!String(reflecting: replacement).contains(key), "debug reveals replacement")
        let replacing = MemoryServiceConnectionEdit(id: "c1", backend: .hindsight, name: "Personal", endpoint: .set("https://memory.example"), secret: replacement)
        let replacementObject = try object(.setConnection(replacing))
        check((replacementObject["secret"] as? [String: Any])?["value"] as? String == key, "replacement trimmed or lost")
        var keeping = replacing
        keeping.secret = .keep
        let kept = try object(.setConnection(keeping))
        check((kept["secret"] as? [String: Any])?["action"] as? String == "keep", "keep encoded incorrectly")
        check((kept["secret"] as? [String: Any])?["value"] == nil, "keep includes key")
        keeping.secret = .clear
        check((try object(.setConnection(keeping))["secret"] as? [String: Any])?["action"] as? String == "clear", "clear encoded incorrectly")
        keeping.secret = .replace("")
        rejects(.invalidSecret) { _ = try MemoryServiceRequest.setConnection(keeping).parameters() }
        keeping.secret = .replace(String(repeating: "é", count: 4097))
        rejects(.invalidSecret) { _ = try MemoryServiceRequest.setConnection(keeping).parameters() }

        let caps = try JSONDecoder().decode(MemoryServiceCapabilities.self, from: Data("{\"recall\":true,\"advanced\":[\"sessions\",\"future_feature\"],\"advanced_actions\":{\"sessions\":[\"list\"]}}".utf8))
        check(caps.supports(.recall), "verified recall hidden")
        check(!caps.supports(.reflect), "missing reflect enabled")
        check(caps.supports(.sessions), "sessions hidden")
        check(!caps.supports(.mentalModels), "unverified mental models enabled")
        check(!MemoryServiceCapabilities().supports(.recall), "empty capabilities enable recall")
        let absentActions = try JSONDecoder().decode(MemoryServiceCapabilities.self, from: Data("{\"advanced\":[\"sessions\",\"documents\"]}".utf8))
        check(!absentActions.supports(.sessions) && !absentActions.supports(.documents), "missing verbs enables advanced actions")
        let gated = try JSONDecoder().decode(MemoryServiceCapabilities.self, from: Data("{\"advanced\":[\"documents\",\"bank_config\"],\"advanced_actions\":{\"documents\":[\"delete\"],\"bank_config\":[\"get\",\"future_verb\"]}}".utf8))
        check(!gated.supports(.documents) && !gated.supports(.documents, verb: "delete"), "document delete bypasses fenced RPC")
        check(gated.supports(.bankConfig, verb: "get") && !gated.supports(.bankConfig, verb: "update") && gated.supports(.bankConfig, verb: "future_verb"), "negotiated verb set is not authoritative")

        var patch = MemoryServiceConnectionEdit(id: "c1", backend: .pgvector, name: "Personal")
        let unchanged = try object(.setConnection(patch))
        check(unchanged["endpoint"] == nil && unchanged["embedding_profile"] == nil && unchanged["options"] == nil, "omission does not preserve")
        patch.endpoint = .clear; patch.embeddingProfile = .clear
        let cleared = try object(.setConnection(patch))
        check(cleared["endpoint"] is NSNull && cleared["embedding_profile"] is NSNull, "clear lost explicit null")
        patch.options = .hindsight
        rejects(.backendOptionsMismatch) { _ = try MemoryServiceRequest.setConnection(patch).parameters() }
        patch = .init(id: "viking", backend: .openViking, name: "Viking", options: .openViking(bindings: [
            "bot-exact": .set(.init(mode: .userKey, accountID: "account", userID: "user", secret: .keep))
        ]))
        let options = try object(.setConnection(patch))["options"] as! [String: Any]
        let binding = (options["bindings"] as! [String: [String: Any]])["bot-exact"]!
        check(binding["api_key"] == nil, "raw key field allowed")
        check((binding["secret"] as? [String: Any])?["action"] as? String == "keep", "binding patch wrong")
        check(options["replace_all"] as? Bool == false, "patch replaces omitted bindings")
        patch.options = .openViking(bindings: ["bot-remove": .remove])
        let removing = try object(.setConnection(patch))["options"] as! [String: Any]
        check((removing["bindings"] as! [String: Any])["bot-remove"] is NSNull, "authorization removal is not explicit null")
        patch.options = .openViking(bindings: [:], replaceAll: true)
        rejects(.confirmationRequired) { _ = try MemoryServiceRequest.setConnection(patch).parameters() }
        patch = .init(id: "pg", backend: .pgvector, name: "Postgres", options: .pgvector(schema: "memory", role: "exact_role"))
        check((try object(.setConnection(patch))["options"] as! [String: Any])["role"] as? String == "exact_role", "typed role lost")
        patch.options = .pgvector(schema: "memory", role: nil)
        check((try object(.setConnection(patch))["options"] as! [String: Any])["role"] is NSNull, "typed null role lost")
        let health = try JSONDecoder().decode(MemoryServiceHealth.self, from: Data("{\"status\":\"setup_required\",\"capabilities\":{},\"deletion_pending\":true}".utf8))
        check(health.status == .setupRequired && health.deletionPending && !health.capabilities.supports(.retain), "fake ready/erase status")
        let operations = try JSONDecoder().decode(MemoryServiceOperations.self, from: Data("{\"operations\":[{\"id\":\"d1\",\"document_id\":\"doc1\",\"state\":\"delivery_unknown\",\"operation_id\":null,\"error_code\":\"lost_reply\",\"document\":\"private-text\"}],\"deletion\":{\"pending\":true,\"operation_id\":null,\"deletion_epoch\":3}}".utf8))
        check(operations.operations[0].state == .deliveryUnknown && operations.deletion?.pending == true, "uncertain delivery treated as stored/erased")
        let connections = try JSONDecoder().decode(MemoryServiceConnections.self, from: Data("{\"schema_version\":1,\"connections\":[{\"id\":\"a\",\"revision\":{\"counter\":2,\"device_id\":\"device-a\"},\"backend\":\"pgvector\",\"name\":\"Personal\",\"has_secret\":true,\"embedding_profile\":null,\"availability\":\"supported\",\"reason\":null,\"secret\":\"must-not-show\",\"endpoint\":\"postgres://private\"}],\"embeddings\":[],\"bots\":[]}".utf8))
        check(connections.connections[0].hasSecret && connections.connections[0].id == "a", "masked config lost")
        check(!String(reflecting: connections).contains("must-not-show"), "unknown secret retained")
        let masked: [String: Any] = ["id": "a", "revision": ["counter": 2, "device_id": "device-a"], "backend": "lance_db", "name": "Neutral name", "has_secret": false, "embedding_profile": NSNull(), "availability": "supported", "reason": NSNull()]
        for key in ["availability", "reason"] {
            var malformed = masked; malformed.removeValue(forKey: key)
            do {
                _ = try JSONDecoder().decode(MemoryServiceConnectionView.self, from: JSONSerialization.data(withJSONObject: malformed))
                fatalError("missing availability metadata accepted")
            } catch is DecodingError { }
        }
        let preferenceData = Data("{\"connection_id\":null,\"auto_recall\":false,\"capture_conversation\":false,\"capture_group_text\":false,\"unattended_capture\":false,\"max_capture_deliveries_per_turn\":1,\"recall_budget\":{\"timeout_ms\":2000,\"max_bytes\":16384,\"max_results\":8,\"max_context_chars\":4000},\"consent_revision\":{\"counter\":1,\"device_id\":\"fixture\"},\"deletion_epoch\":0}".utf8)
        let preferencesView = try JSONDecoder().decode(MemoryServicePreferencesView.self, from: preferenceData)
        check(preferencesView.editable.connectionID == nil && !preferencesView.editable.captureConversation, "masked preferences changed defaults")
        var missingBinding = try JSONSerialization.jsonObject(with: preferenceData) as! [String: Any]
        missingBinding.removeValue(forKey: "connection_id")
        do {
            _ = try JSONDecoder().decode(MemoryServicePreferencesView.self, from: JSONSerialization.data(withJSONObject: missingBinding))
            fatalError("missing binding decoded as explicit disconnect")
        } catch is DecodingError { }

        var calls: [String] = []
        let api = MemoryServiceAPI { method, _ in calls.append(method); return Data("null".utf8) }
        let invalid = MemoryServicePreferencesDraft(botID: "invalid", saved: .init(connectionID: "a", unattendedCapture: true))
        do { _ = try await api.savePreferences(invalid); fatalError("invalid draft sent") }
        catch let error as MemoryServiceDraftError { check(error == .enforcedSpendingPolicyRequired, "unexpected refusal") }
        check(calls.isEmpty, "refusal reached RPC")
        _ = try await api.savePreferences(MemoryServicePreferencesDraft(botID: "bot-a"))
        check(calls == ["memory.preferences.set"], "wrong mutation RPC")
        print("MemoryServiceDraft: consent, secret, capability, budget and fixture RPC checks passed")
    }
}
