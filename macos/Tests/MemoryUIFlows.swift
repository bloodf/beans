import AppKit

@MainActor
func runMemoryUIFlows() async throws -> [MemoryFormViewController] {
    func check(_ condition: Bool, _ message: String) { if !condition { fatalError(message) } }
    func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
    func button(_ title: String, _ controller: MemoryFormViewController) -> NSButton {
        controller.controls(in: controller.view).compactMap { $0 as? NSButton }.first { $0.title == title }!
    }
    func data(_ value: Any) throws -> Data { try JSONSerialization.data(withJSONObject: value) }
    let revision: [String: Any] = ["counter": 1, "device_id": "fixture"]
    var deletionEpoch: UInt64 = 0
    func preferences(capture: Bool) throws -> Data {
        try data(["connection_id": "c", "auto_recall": capture, "capture_conversation": capture, "capture_group_text": capture, "unattended_capture": false, "max_capture_deliveries_per_turn": 1, "recall_budget": ["timeout_ms": 2000, "max_bytes": 16384, "max_results": 8, "max_context_chars": 4000], "consent_revision": revision, "deletion_epoch": deletionEpoch])
    }
    let profile: [String: Any] = ["id": "local", "revision": revision, "model": "fixture-model", "model_revision": "fixture-v1", "dimensions": 3, "has_secret": false]
    let listing = try data(["schema_version": 1, "connections": [["id": "c", "revision": revision, "backend": "pgvector", "name": "Fixture database", "has_secret": true, "embedding_profile": "local", "availability": "supported", "reason": NSNull()]], "embeddings": [profile], "bots": []])
    let healthData = Data("{\"status\":\"ready\",\"capabilities\":{\"retain\":true,\"recall\":true,\"inspect\":true,\"clear\":true,\"delete_document\":true,\"operation_status\":true,\"advanced\":[\"memory_edit\"],\"advanced_actions\":{\"memory_edit\":[\"edit\"]}},\"deletion_pending\":false}".utf8)
    let unknown = Data("{\"id\":\"delivery\",\"document_id\":\"document\",\"state\":\"delivery_unknown\",\"operation_id\":null,\"error_code\":\"lost_reply\"}".utf8)
    var calls: [(String, [String: Any])] = []
    var captured = false
    var stored = false
    var failPreferences = false
    var failDeleteAfterFence = false
    var failAuthorityConnections = false, failAuthorityPreferences = false
    let service = MemoryServiceAPI { method, body in
        let object = try JSONSerialization.jsonObject(with: body) as! [String: Any]
        calls.append((method, object))
        switch method {
        case "memory.connections.list":
            if failAuthorityConnections { throw MemoryUIError.unavailable }
            return listing
        case "memory.preferences.get":
            if failAuthorityPreferences { throw MemoryUIError.unavailable }
            return try preferences(capture: captured)
        case "memory.preferences.set":
            if failPreferences { throw MemoryUIError.unavailable }
            captured = true; return try preferences(capture: true)
        case "memory.service.health": return healthData
        case "memory.service.retain":
            return stored ? Data("{\"id\":\"delivery\",\"document_id\":\"document\",\"state\":\"completed\",\"operation_id\":null,\"error_code\":null}".utf8) : unknown
        case "memory.operations.list": return Data("{\"operations\":[{\"id\":\"old-delivery\",\"document_id\":\"old-document\",\"state\":\"failed\",\"operation_id\":null,\"error_code\":\"fixture_failure\"}],\"deletion\":{\"pending\":false,\"operation_id\":null,\"deletion_epoch\":0}}".utf8)
        case "memory.service.delete":
            guard (object["deletion_epoch"] as? NSNumber)?.uint64Value == deletionEpoch else { throw MemoryUIError.staleTarget }
            deletionEpoch += 1
            if failDeleteAfterFence { throw MemoryUIError.unavailable }
            return try data(["pending": true, "deletion_epoch": deletionEpoch, "operation_id": NSNull()])
        default: throw MemoryUIError.unsupported
        }
    }
    let target: [String: Any] = ["database": "fixture", "database_oid": 42, "role": "test_role", "server_address": "127.0.0.1", "server_port": 5432, "server_version": "17", "session_pid": 22]
    let readiness: [String: Any] = ["ready": false, "extension_version": NSNull(), "extension_schema": NSNull(), "schema_exists": false, "can_create_schema": true, "can_create_tables": true, "can_install_extension": false]
    let sql = "-- fixture: approval text only, never executed\nSELECT 1;"
    let preview = try data(["token": "fixture-token", "expires_at": Date().timeIntervalSince1970 + 300.5, "action": "memory.pgvector.initialize.preview", "runner_id": "runner", "bot_id": "bot", "profile_id": NSNull(), "profile_revision": NSNull(), "connection_revision": revision, "details": ["target": target, "schema": "fixture_schema", "sql": sql, "readiness": readiness]])
    let assets: [MemoryAssetRequest] = [.runtime, .model, .tokenizer].map { .init(kind: $0, source: .supplied(path: "/fixture/\($0.rawValue)"), license: "Fixture license", bytes: 10, sha256: String(repeating: "a", count: 64)) }
    let assetReply = try data(["preview_token": "asset-token", "preview_digest": String(repeating: "b", count: 64), "expires_in_seconds": 600, "runner_id": "runner", "profile_id": "local", "profile_revision": revision, "total_bytes": 30, "assets": JSONSerialization.jsonObject(with: JSONEncoder().encode(assets))])
    var setupCalls: [(String, [String: Any])] = []
    let setup = MemorySetupAPI { method, body in
        setupCalls.append((method, try JSONSerialization.jsonObject(with: body) as! [String: Any]))
        switch method {
        case "memory.pgvector.initialize.preview": return preview
        case "memory.embeddings.local.preview": return assetReply
        case "memory.pgvector.initialize.apply", "memory.embeddings.local.apply": throw MemoryUIError.unavailable
        default: throw MemoryUIError.unsupported
        }
    }
    var callbackFailures: [String] = []
    let edited = BotMemoryServiceViewController(botID: "draft-bot", name: "Draft fixture", runnerID: "runner", service: service, setup: setup, currentRunner: { "runner" })
    edited.loadViewIfNeeded(); edited.reloadPreferences(); await edited.pendingTask?.value
    edited.capture.state = .on; edited.autoRecall.state = .on; edited.groupCapture.state = .on
    captured = true; edited.reloadPreferences(); await edited.pendingTask?.value
    captured = false; edited.reloadPreferences(); await edited.pendingTask?.value
    if edited.capture.state != .on || edited.autoRecall.state != .on || edited.groupCapture.state != .on {
        callbackFailures.append("two authoritative refreshes replace the unsaved editable baseline")
    }
    let fenced = BotMemoryServiceViewController(botID: "fenced-bot", name: "Fence fixture", runnerID: "runner", service: service, setup: setup, currentRunner: { "runner" })
    fenced.loadViewIfNeeded(); fenced.reloadPreferences(); await fenced.pendingTask?.value
    fenced.confirmationHandler = { _, _ in true }
    button("Check service health / capabilities", fenced).performClick(nil); await fenced.pendingTask?.value
    let fencedText = descendants(fenced.view).compactMap { $0 as? NSTextView }.first { $0.accessibilityLabel() == "Explicit memory text" }!
    fencedText.string = "unsaved fixture memory"
    failDeleteAfterFence = true
    button("Clear this bot's service bank…", fenced).performClick(nil); await fenced.pendingTask?.value
    let lockedTitles = ["Save explicit service memory…", "Search this bot's memory…", "Inspect document and source provenance", "Delete this document…", "Clear this bot's service bank…", "Check service health / capabilities", "Load operations / deletion status", "Preview selected setup…"]
    if lockedTitles.contains(where: { button($0, fenced).isEnabled }) || fenced.confirmButton.isEnabled {
        callbackFailures.append("delete failure after the fence restores stale service actions")
    }
    check(callbackFailures.isEmpty, callbackFailures.joined(separator: "\n"))
    check(fenced.canDismiss && fencedText.string == "unsaved fixture memory", "uncertain deletion traps or discards the draft")
    let lockedCallCount = calls.count
    for title in lockedTitles {
        (button(title, fenced) as! MemoryActionButton).handler?(); await fenced.pendingTask?.value
    }
    check(calls.count == lockedCallCount, "locked callback still reaches service transport")
    failDeleteAfterFence = false; failAuthorityPreferences = true
    fenced.reloadPreferences(); await fenced.pendingTask?.value
    check(!fenced.retainButton.isEnabled && !button("Preview selected setup…", fenced).isEnabled && !fenced.confirmButton.isEnabled, "partial preferences refresh restores authority")
    failAuthorityPreferences = false; failAuthorityConnections = true
    fenced.reloadPreferences(); await fenced.pendingTask?.value
    check(!fenced.retainButton.isEnabled && !button("Preview selected setup…", fenced).isEnabled && !fenced.confirmButton.isEnabled, "partial connection refresh restores authority")
    failAuthorityConnections = false
    fenced.reloadPreferences(); await fenced.pendingTask?.value
    check(button("Preview selected setup…", fenced).isEnabled && fenced.confirmButton.isEnabled && !fenced.retainButton.isEnabled, "complete authority refresh fails recovery or reuses old health")
    button("Check service health / capabilities", fenced).performClick(nil); await fenced.pendingTask?.value
    check(fenced.retainButton.isEnabled && fencedText.string == "unsaved fixture memory", "explicit health recovery loses draft or remains locked")
    deletionEpoch = 0; calls.removeAll()
    print("PASS: repeated authoritative refresh preserves editable baseline; uncertain deletion locks callbacks through partial refresh and recovers explicitly")
    let bot = BotMemoryServiceViewController(botID: "bot", name: "Fixture", runnerID: "runner", service: service, setup: setup, currentRunner: { "runner" })
    bot.loadViewIfNeeded(); bot.reloadPreferences(); await bot.pendingTask?.value
    check(bot.capture.state == .off && bot.groupCapture.state == .off, "loading bound bot silently enables capture")
    check(!bot.retainButton.isEnabled && !bot.reflectButton.isEnabled, "supported configuration is misrepresented as an initialized service")
    bot.capture.state = .on; bot.groupCapture.state = .on; bot.autoRecall.state = .on
    var consentPrompts: [String] = []
    var allowGroup = false
    bot.confirmationHandler = { title, _ in consentPrompts.append(title); return !title.contains("group") || allowGroup }
    bot.confirmTapped(); await bot.pendingTask?.value
    check(consentPrompts.count == 2 && !calls.contains { $0.0 == "memory.preferences.set" }, "declined group consent still writes preferences")
    allowGroup = true; failPreferences = true
    bot.confirmTapped(); await bot.pendingTask?.value
    check(bot.capture.state == .on && bot.groupCapture.state == .on && bot.canDismiss, "failed preference save loses toggles or remains fenced")
    failPreferences = false
    bot.confirmTapped(); await bot.pendingTask?.value
    let preferenceWrites = calls.filter { $0.0 == "memory.preferences.set" }
    check(preferenceWrites.count == 2 && preferenceWrites.last!.1["bot_id"] as? String == "bot", "preference failure recovery writes wrong bot")
    check(preferenceWrites.last!.1["unattended_capture"] as? Bool == false, "capture enables unbounded worker")
    button("Check service health / capabilities", bot).performClick(nil); await bot.pendingTask?.value
    check(bot.retainButton.isEnabled && !bot.reflectButton.isEnabled && bot.reflectButton.isHidden, "negotiated pgvector shows native reflection")
    let editor = descendants(bot.view).compactMap { $0 as? NSTextView }.first { $0.accessibilityLabel() == "Explicit memory text" }!
    editor.string = "synthetic fixture memory"
    var result = ""; bot.resultHandler = { result = $0 }
    bot.retainButton.performClick(nil)
    check(!editor.isEditable && !bot.canDismiss, "in-flight save permits draft edits or dismissal")
    await bot.pendingTask?.value
    check(editor.isEditable, "failed/uncertain retain leaves text editor disabled")
    check(result.contains("Not confirmed stored") && !editor.string.isEmpty, "uncertain delivery claimed stored or discarded text")
    stored = true; bot.retainButton.performClick(nil); await bot.pendingTask?.value
    let retains = calls.filter { $0.0 == "memory.service.retain" }
    check(retains.count == 2 && retains[0].1["request_id"] as? String == retains[1].1["request_id"] as? String, "unchanged explicit-save retry changes request identity")
    check(editor.string.isEmpty && result.hasPrefix("Stored"), "confirmed stored text remains as unsaved draft")
    bot.capture.state = .off; bot.groupCapture.state = .off
    let timeout = bot.controls(in: bot.view).compactMap { $0 as? NSTextField }.first { $0.accessibilityLabel() == "Timeout milliseconds (1–5000)" }!
    timeout.stringValue = "invalid draft"
    bot.reloadPreferences(); await bot.pendingTask?.value
    check(bot.capture.state == .off && bot.groupCapture.state == .off && timeout.stringValue == "invalid draft", "masked preference refresh discards dirty or invalid input")
    timeout.stringValue = "2000"
    button("Check service health / capabilities", bot).performClick(nil); await bot.pendingTask?.value
    button("Load operations / deletion status", bot).performClick(nil); await bot.pendingTask?.value
    check(button("Retry selected delivery…", bot).isEnabled, "failed current-epoch delivery cannot be retried")
    button("Clear this bot's service bank…", bot).performClick(nil); await bot.pendingTask?.value
    check(!button("Retry selected delivery…", bot).isEnabled && !button("Check selected operation", bot).isEnabled, "deletion epoch leaves stale operation actions unlocked")
    check(!bot.retainButton.isEnabled && !button("Clear this bot's service bank…", bot).isEnabled && !bot.confirmButton.isEnabled, "delete acknowledgment bypasses authority recovery")
    button("Refresh preferences and connection", bot).performClick(nil); await bot.pendingTask?.value
    button("Check service health / capabilities", bot).performClick(nil); await bot.pendingTask?.value
    button("Clear this bot's service bank…", bot).performClick(nil); await bot.pendingTask?.value
    let deletes = calls.filter { $0.0 == "memory.service.delete" }
    check(deletes.count == 2 && (deletes[0].1["deletion_epoch"] as? NSNumber)?.uint64Value == 0 && (deletes[1].1["deletion_epoch"] as? NSNumber)?.uint64Value == 1, "next deletion reuses a stale confirmation epoch")
    check(bot.capture.state == .off && bot.groupCapture.state == .off, "deletion refresh discards dirty capture preferences")

    var runner = "runner"
    let sqlForm = MemoryBotSetupViewController(service: service, setup: setup, botID: "bot", connectionID: "c", action: .pgvector, currentRunner: { runner })
    sqlForm.loadViewIfNeeded(); sqlForm.confirmationHandler = { _, details in check(details.contains(sql) && details.contains("Can install extension: false") && details.contains("test_role"), "exact SQL/actual role/readiness omitted from approval"); return true }
    sqlForm.preview(); await sqlForm.pendingTask?.value
    button("I approve this exact target and previewed changes", sqlForm).performClick(nil)
    runner = "changed"
    sqlForm.confirmTapped(); await sqlForm.pendingTask?.value
    check(!setupCalls.contains { $0.0 == "memory.pgvector.initialize.apply" }, "changed Runner accepts old preview")
    runner = "runner"; sqlForm.preview(); await sqlForm.pendingTask?.value
    button("I approve this exact target and previewed changes", sqlForm).performClick(nil)
    sqlForm.confirmTapped(); await sqlForm.pendingTask?.value
    let applyCount = setupCalls.filter { $0.0 == "memory.pgvector.initialize.apply" }.count
    sqlForm.confirmTapped(); await sqlForm.pendingTask?.value
    check(applyCount == 1 && setupCalls.filter { $0.0 == "memory.pgvector.initialize.apply" }.count == 1 && !sqlForm.confirmButton.isEnabled, "failed apply permits second side effect")
    let sqlApply = setupCalls.first { $0.0 == "memory.pgvector.initialize.apply" }!.1
    check(sqlApply["sql"] == nil && sqlApply["role"] == nil && sqlApply["token"] as? String == "fixture-token", "apply substitutes SQL/role instead of exact approval")

    let localProfile = try JSONDecoder().decode(MemoryServiceEmbeddingView.self, from: data(profile))
    let localForm = MemoryLocalAssetsViewController(service: service, setup: setup, profile: localProfile, runners: [("runner", "Fixture Runner")])
    localForm.loadViewIfNeeded(); localForm.confirmationHandler = { _, details in check(details.contains("Fixture license") && details.contains(String(repeating: "a", count: 64)), "asset consent omits license/hash"); return true }
    for kind in ["runtime", "model", "tokenizer"] {
        let controls = localForm.controls(in: localForm.view).compactMap { $0 as? NSTextField }
        controls.first { $0.accessibilityLabel() == "\(kind) Exact path / URL" }!.stringValue = "/fixture/\(kind)"
        controls.first { $0.accessibilityLabel() == "\(kind) License" }!.stringValue = "Fixture license"
        controls.first { $0.accessibilityLabel() == "\(kind) Exact byte count" }!.stringValue = "10"
        controls.first { $0.accessibilityLabel() == "\(kind) Full lowercase SHA-256" }!.stringValue = String(repeating: "a", count: 64)
    }
    localForm.preview(); await localForm.pendingTask?.value
    let assetFields = localForm.controls(in: localForm.view).compactMap { $0 as? NSTextField }.filter { $0.accessibilityLabel()?.hasSuffix("Exact path / URL") == true }
    check(assetFields.allSatisfy { !$0.isEnabled }, "previewed asset fields remain mutable")
    button("I approve these exact assets, licenses and runtime load", localForm).performClick(nil)
    localForm.confirmTapped(); await localForm.pendingTask?.value
    localForm.confirmTapped(); await localForm.pendingTask?.value
    check(setupCalls.filter { $0.0 == "memory.embeddings.local.apply" }.count == 1, "failed asset acquisition permits reuse")
    check(assetFields.allSatisfy { $0.isEnabled }, "failed asset apply traps editable draft")

    let hindsight = MemoryAdvancedViewController(service: service, botID: "bot", backend: .hindsight, capabilities: try JSONDecoder().decode(MemoryServiceCapabilities.self, from: Data("{\"advanced\":[\"bank_config\",\"mental_models\",\"directives\",\"documents\"],\"advanced_actions\":{\"bank_config\":[\"get\",\"update\"],\"mental_models\":[\"list\",\"get\",\"create\",\"update\",\"refresh\",\"history\",\"delete\"],\"directives\":[\"list\",\"get\",\"create\",\"update\",\"delete\"],\"documents\":[\"list\",\"get\",\"chunks\"]}}".utf8)))
    hindsight.loadViewIfNeeded(); hindsight.select(feature: .mentalModels, action: "create")
    hindsight.fields["name"]!.stringValue = "Fixture model"; hindsight.fields["source_query"]!.stringValue = "synthetic query"
    let model = try hindsight.command()
    check((model.body["trigger"] as? [String: Any])?["refresh_after_consolidation"] as? Bool == false, "mental model enables background consolidation refresh")
    hindsight.select(feature: .bankConfig, action: "update")
    hindsight.fields["reflect_mission"]!.stringValue = "Fixture mission"
    let update = try hindsight.command().body["updates"] as! [String: Any]
    check(update["retain_extract_labels"] == nil && update["recall_budget"] == nil, "partial config edit changes untouched options")
    hindsight.fields["disposition_empathy"]!.stringValue = "1.5"
    do { _ = try hindsight.command(); fatalError("fractional integer disposition accepted") } catch MemoryUIError.invalidInput { }
    hindsight.fields["disposition_empathy"]!.stringValue = "3"
    let vector = MemoryAdvancedViewController(service: service, botID: "bot", backend: .pgvector, capabilities: try JSONDecoder().decode(MemoryServiceCapabilities.self, from: healthData.componentsCapabilities()))
    vector.loadViewIfNeeded(); vector.select(feature: .memoryEdit, action: "edit")
    vector.fields["document_id"]!.stringValue = "document"; vector.fields["text"]!.stringValue = "replacement"
    check(try vector.command().body["document_id"] as? String == "document", "vector edit uses Hindsight memory-ID shape")
    let legacyCaps = try JSONDecoder().decode(MemoryServiceCapabilities.self, from: Data("{\"advanced\":[\"sessions\",\"documents\"]}".utf8))
    check(MemoryAdvancedViewController.features(backend: .openViking, capabilities: legacyCaps).isEmpty, "legacy feature-only reply exposes actions")
    let deleteOnly = try JSONDecoder().decode(MemoryServiceCapabilities.self, from: Data("{\"advanced\":[\"documents\"],\"advanced_actions\":{\"documents\":[\"delete\"]}}".utf8))
    check(MemoryAdvancedViewController.features(backend: .hindsight, capabilities: deleteOnly).isEmpty, "advanced document delete bypasses fenced route")
    let unavailableAdvanced = MemoryAdvancedViewController(service: service, botID: "bot", backend: .openViking, capabilities: legacyCaps)
    unavailableAdvanced.loadViewIfNeeded()
    let beforeUnsupported = calls.count
    unavailableAdvanced.confirmTapped()
    check(calls.count == beforeUnsupported && !unavailableAdvanced.confirmButton.isEnabled && unavailableAdvanced.errorLabel.stringValue.contains("unsupported"), "unsupported native action sends an RPC")
    check(!MemoryUIRPC.allowed.contains("memory.embeddings.assets.apply") && !MemoryUIRPC.allowed.contains("memory.embeddings.download.apply"), "superseded setup methods accepted")
    print("PASS: native group refusal, failed draft recovery, pending retain, target drift, exact SQL/role/readiness, failed-apply one-use, asset freeze, advanced edit boundaries")
    let account = MemorySettingsViewController(service: service, setup: setup, bots: [("bot", "Fixture")], runners: [("runner", "Fixture Runner")])
    account.loadViewIfNeeded(); account.reload(); await account.pendingTask?.value
    return [account, bot, sqlForm, localForm, hindsight, vector]
}

private extension Data {
    func componentsCapabilities() throws -> Data {
        let object = try JSONSerialization.jsonObject(with: self) as! [String: Any]
        return try JSONSerialization.data(withJSONObject: object["capabilities"]!)
    }
}
