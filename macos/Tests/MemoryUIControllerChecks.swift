import AppKit

func L(_ text: String, _ arguments: CVarArg...) -> String { arguments.isEmpty ? text : String(format: text, arguments: arguments) }
enum Theme { enum Font { static let caption = NSFont.systemFont(ofSize: 11); static let body = NSFont.systemFont(ofSize: 13) } }
struct Bot { let id: String, name: String, runnerID: String }
struct Device { enum Status { case online, pairing, offline }; let id: String, name: String }
@MainActor final class AppStore {
    static let shared = AppStore()
    let bots: [Bot] = []; let runners: [Device] = []
    var memoryService: MemoryServiceAPI { MemoryServiceAPI { _, _ in throw MemoryUIError.unavailable } }
    var memorySetup: MemorySetupAPI { MemorySetupAPI { _, _ in throw MemoryUIError.unavailable } }
    func bot(_ id: String) -> Bot? { nil }
}
private func check(_ condition: Bool, _ message: String) { if !condition { fatalError(message) } }

@main struct MemoryUIControllerChecks {
    @MainActor static func main() async throws {
        _ = NSApplication.shared
        var requests: [(String, [String: Any])] = []
        var failSave = true
        let api = MemoryServiceAPI { method, data in
            let object = try JSONSerialization.jsonObject(with: data) as! [String: Any]
            requests.append((method, object))
            if failSave { throw MemoryUIError.unavailable }
            return Data("{\"saved\":true}".utf8)
        }
        let form = MemoryConnectionViewController(service: api, saved: nil, embeddings: [], bots: [])
        form.loadViewIfNeeded()
        form.nameField.stringValue = "Private notes"
        form.endpointMode.selectItem(at: 1)
        form.endpointField.stringValue = "https://fixture.invalid"
        form.secret.mode.selectItem(at: 1)
        form.secret.field.stringValue = " exact secret "
        form.confirmTapped()
        check(!form.canDismiss && !form.confirmButton.isEnabled, "save does not fence dismissal")
        form.cancelOperation(nil)
        await form.pendingTask?.value
        check(form.canDismiss && form.nameField.stringValue == "Private notes", "failed save discarded draft")
        check(form.secret.field.stringValue == " exact secret ", "failed save discarded secret")
        check(form.errorLabel.stringValue.contains("unavailable"), "failure not visible")
        failSave = false
        form.confirmTapped()
        await form.pendingTask?.value
        let saved = requests.last!.1
        check((saved["secret"] as? [String: Any])?["value"] as? String == " exact secret ", "secret bytes changed")
        check(requests[0].1["id"] as? String == requests[1].1["id"] as? String, "failed connection save changes stable ID")
        let fieldWindow = NSWindow(contentViewController: form)
        check(fieldWindow.makeFirstResponder(form.nameField) && form.nameField.currentEditor() != nil, "native name field cannot receive editing focus")
        fieldWindow.makeFirstResponder(nil); fieldWindow.contentViewController = nil
        let lance = MemoryConnectionViewController(service: api, saved: nil, embeddings: [], bots: [])
        lance.loadViewIfNeeded(); lance.nameField.stringValue = "Local fixture"
        lance.backendMenu.selectItem(at: 3)
        NSApplication.shared.sendAction(lance.backendMenu.action!, to: lance.backendMenu.target, from: lance.backendMenu)
        let localEdit = try JSONSerialization.jsonObject(with: MemoryServiceRequest.setConnection(lance.edit()).parameters()) as! [String: Any]
        check(localEdit["endpoint"] is NSNull, "local Lance silently selects a Cloud endpoint")
        let beforeCloud = requests.count
        lance.endpointMode.selectItem(at: 1); lance.endpointField.stringValue = "db://fixture"
        lance.confirmTapped()
        check(requests.count == beforeCloud && lance.errorLabel.stringValue.contains("lancedb_cloud_transport_unavailable") && lance.nameField.stringValue == "Local fixture", "Cloud activation sends an RPC or loses the local draft")
        let blockedListing = Data("""
        {"schema_version":1,"connections":[{"id":"blocked","revision":{"counter":1,"device_id":"fixture"},"backend":"lance_db","name":"Neutral name","has_secret":false,"embedding_profile":null,"availability":"blocked","reason":"lancedb_cloud_transport_unavailable"}],"embeddings":[],"bots":[]}
        """.utf8)
        var blockedCalls: [String] = []
        let blockedAPI = MemoryServiceAPI { method, _ in
            blockedCalls.append(method)
            if method == "memory.connections.list" { return blockedListing }
            if method == "memory.preferences.get" {
                return Data("""
                {"connection_id":"blocked","auto_recall":false,"capture_conversation":false,"capture_group_text":false,"unattended_capture":false,"max_capture_deliveries_per_turn":1,"recall_budget":{"timeout_ms":2000,"max_bytes":16384,"max_results":8,"max_context_chars":4000},"consent_revision":{"counter":1,"device_id":"fixture"},"deletion_epoch":0}
                """.utf8)
            }
            throw MemoryUIError.unavailable
        }
        let blockedBot = BotMemoryServiceViewController(botID: "bot", name: "Fixture", runnerID: "runner", service: blockedAPI, setup: MemorySetupAPI { _, _ in throw MemoryUIError.unavailable }, currentRunner: { "runner" })
        blockedBot.loadViewIfNeeded(); blockedBot.reloadPreferences(); await blockedBot.pendingTask?.value
        let healthButton = blockedBot.controls(in: blockedBot.view).compactMap { $0 as? NSButton }.first { $0.title == "Check service health / capabilities" }!
        healthButton.performClick(nil); await blockedBot.pendingTask?.value
        blockedBot.autoRecall.state = .on; blockedBot.confirmationHandler = { _, _ in true }
        blockedBot.confirmTapped(); await blockedBot.pendingTask?.value
        check(!blockedCalls.contains("memory.service.health") && !blockedCalls.contains("memory.preferences.set") && blockedBot.canDismiss, "masked blocked Cloud probes, activates, or remains busy")
        check(blockedBot.errorLabel.stringValue.contains("lancedb_cloud_transport_unavailable"), "blocked Cloud loses its actionable reason")
        let blockedSaved = try JSONDecoder().decode(MemoryServiceConnections.self, from: blockedListing).connections[0]
        let blockedEdit = MemoryConnectionViewController(service: blockedAPI, saved: blockedSaved, embeddings: [], bots: [])
        blockedEdit.loadViewIfNeeded(); blockedEdit.nameField.stringValue = "Renamed configuration"
        let preserved = try JSONSerialization.jsonObject(with: MemoryServiceRequest.setConnection(blockedEdit.edit()).parameters()) as! [String: Any]
        check(preserved["endpoint"] == nil && preserved["options"] == nil && (preserved["secret"] as? [String: Any])?["action"] as? String == "keep", "masked blocked edit changes saved private configuration")
        var blockedSetupCalls: [String] = []
        let blockedSetup = MemoryBotSetupViewController(service: blockedAPI, setup: MemorySetupAPI { method, _ in blockedSetupCalls.append(method); throw MemoryUIError.unavailable }, botID: "bot", connectionID: "blocked", action: .lanceBinding, currentRunner: { "runner" })
        blockedSetup.loadViewIfNeeded()
        blockedSetup.controls(in: blockedSetup.view).compactMap { $0 as? NSTextField }.first { $0.accessibilityLabel() == "Exact absolute directory on assigned Runner" }!.stringValue = "/fixture/local"
        blockedSetup.preview(); await blockedSetup.pendingTask?.value
        check(blockedSetupCalls.isEmpty && blockedSetup.canDismiss && blockedSetup.errorLabel.stringValue.contains("lancedb_cloud_transport_unavailable"), "blocked Cloud reaches setup transport or leaves its controller busy")

        var remote = MemoryServicePreferencesDraft(botID: "bot")
        remote.connectionID = "c"; remote.captureConversation = true; remote.captureGroupText = true
        do { _ = try remote.request(); fatalError("capture accepted without consent") } catch MemoryServiceDraftError.plaintextConsentRequired { }
        remote.approveRemotePlaintext()
        do { _ = try remote.request(); fatalError("group accepted without consent") } catch MemoryServiceDraftError.groupConsentRequired { }
        remote.approveGroupCapture(); _ = try remote.request()

        let advanced = MemoryAdvancedViewController(service: api, botID: "bot", backend: .openViking, capabilities: try JSONDecoder().decode(MemoryServiceCapabilities.self, from: Data("{\"advanced\":[\"sessions\",\"resources\",\"tasks\"],\"advanced_actions\":{\"sessions\":[\"list\",\"create\",\"get\",\"add_message\",\"commit\",\"delete\"],\"resources\":[\"list\",\"add\",\"read\",\"delete\"],\"tasks\":[\"list\",\"get\",\"cancel\"]}}".utf8)))
        advanced.loadViewIfNeeded()
        advanced.select(feature: .sessions, action: "add_message")
        advanced.fields["id"]?.stringValue = "session1"
        advanced.fields["content"]?.stringValue = "synthetic text"
        let command = try advanced.command()
        check(command.body["role"] as? String == "user", "OpenViking role is not typed")
        check(command.body["bank_id"] == nil && command.body["uri"] == nil, "advanced scope selector exposed")
        advanced.select(feature: .resources, action: "read")
        advanced.fields["path"]?.stringValue = "resources/../other"
        do { _ = try advanced.command(); fatalError("escaping resource path accepted") } catch MemoryUIError.invalidInput { }

        let setup = MemorySetupAPI { _, _ in throw MemoryUIError.unavailable }
        let botForm = BotMemoryServiceViewController(botID: "bot", name: "Fixture", runnerID: "runner", service: api, setup: setup, currentRunner: { "runner" })
        botForm.loadViewIfNeeded()
        check(botForm.capture.state == .off && botForm.groupCapture.state == .off, "new capture enabled")
        check(!botForm.retainButton.isEnabled && !botForm.reflectButton.isEnabled, "unnegotiated capabilities enabled")
        let flowForms = try await runMemoryUIFlows()
        let forms: [MemoryFormViewController] = [form, botForm, advanced, MemoryEmbeddingViewController(service: api, saved: nil)] + flowForms
        for controller in forms {
            controller.loadViewIfNeeded()
            for appearance in [NSAppearance.Name.aqua, .darkAqua] {
                let window = NSWindow(contentViewController: controller)
                window.appearance = NSAppearance(named: appearance)
                window.setContentSize(controller.view.fittingSize)
                window.contentView?.layoutSubtreeIfNeeded()
                check(controller.formScroll.frame.height <= 460, "form grows beyond scroll viewport")
                check(controller.formColumn.frame.width <= controller.formScroll.contentSize.width + 1, "form scroll width overflow")
                window.contentViewController = nil
            }
        }
        print("PASS: guarded native drafts, failure recovery, exact secrets, capability/consent gates, typed OpenViking, light/dark scroll sizing")
        // AppKit's shared application can keep an async command-line main alive after all
        // checks return. This fixture owns its process; terminate after completed proof.
        exit(0)
    }
}
