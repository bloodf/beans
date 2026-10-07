import AppKit

final class BotMemoryServiceViewController: MemoryFormViewController {
    private let service: MemoryServiceAPI, setup: MemorySetupAPI
    let botID: String
    private let runnerID: String, currentRunner: () -> String?
    private var inventory: MemoryServiceConnections?, saved: MemoryServicePreferencesView?
    private var editableBaseline: MemoryServicePreferences?
    private var authorityRecovered = false
    private var draft: MemoryServicePreferencesDraft
    private var health: MemoryServiceHealth?
    private var operations: MemoryServiceOperations?
    private let connectionMenu = NSPopUpButton()
    let autoRecall = NSButton(checkboxWithTitle: L("Automatic recall before normal turns"), target: nil, action: nil)
    let capture = NSButton(checkboxWithTitle: L("Capture future completed conversation text"), target: nil, action: nil)
    let groupCapture = NSButton(checkboxWithTitle: L("Also capture future group conversation text"), target: nil, action: nil)
    private var budgets: [String: NSTextField] = [:]
    private let statusLabel = Build.label(L("No negotiated service capabilities. Local MEMORY.md remains usable."), font: .systemFont(ofSize: 12), color: .secondaryLabelColor, lines: 0)
    private let operationSummary = Build.label(L("Load operations to see pending deliveries and deletion."), font: .systemFont(ofSize: 12), color: .secondaryLabelColor, lines: 0)
    private let operationMenu = NSPopUpButton()
    private var query: NSTextField!, documentID: NSTextField!
    private let retainText = NSTextView()
    let retainButton = MemoryActionButton("Save explicit service memory…")
    let reflectButton = MemoryActionButton("Native Hindsight reflect…")
    private var recallButton: MemoryActionButton!, inspectButton: MemoryActionButton!, deleteButton: MemoryActionButton!, clearButton: MemoryActionButton!
    private var statusButton: MemoryActionButton!, cancelOperationButton: MemoryActionButton!, retryButton: MemoryActionButton!, advancedButton: MemoryActionButton!
    private var healthButton: MemoryActionButton!, loadOperationsButton: MemoryActionButton!
    private var setupMenu: NSPopUpButton!, setupButton: MemoryActionButton!
    private var retainedDraft: (text: String, requestID: String)?

    convenience init(bot: Bot) {
        let store = AppStore.shared
        self.init(botID: bot.id, name: bot.name, runnerID: bot.runnerID, service: store.memoryService, setup: store.memorySetup, currentRunner: { store.bot(bot.id)?.runnerID })
    }
    init(botID: String, name: String, runnerID: String, service: MemoryServiceAPI, setup: MemorySetupAPI, currentRunner: @escaping () -> String?) {
        self.botID = botID; self.runnerID = runnerID; self.service = service; self.setup = setup; self.currentRunner = currentRunner
        draft = MemoryServicePreferencesDraft(botID: botID)
        super.init(title: L("%@'s memory service", name), subtitle: L("Service memory is separate from local MEMORY.md. Every operation is bound to this exact bot by core."), width: 620)
    }
    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }
    override func loadView() {
        super.loadView()
        note("Bot: \(botID) · assigned Runner: \(runnerID). Bank scope is derived from the account key and stable bot ID; rename/provider changes retain scope. Namespaces are logical isolation, not server authorization.")
        heading("Connection and consent")
        row("Account memory connection", connectionMenu)
        connectionMenu.addItem(withTitle: L("Disabled — local memory only")); connectionMenu.target = self; connectionMenu.action = #selector(connectionChanged)
        addForm(statusLabel)
        addForm(MemoryActionButton(L("Refresh preferences and connection")) { [weak self] in self?.reloadPreferences() })
        addForm(MemoryActionButton("Account connections…") { [weak self] in self?.presentAsSheet(MemorySettingsViewController()) })
        addForm(autoRecall); addForm(capture); addForm(groupCapture)
        note("Remote recall sends the turn's task text. Capture applies only to future eligible completed user/assistant text, never historical backfill, attachments, hidden thinking or tool payloads. Group text may include other participants and requires separate consent. Scrubbing is best effort, not DLP.")
        note("Plaintext reaches the configured memory endpoint and its downstream extraction, embedding or reranking models outside the encrypted relay. Those services may charge you. Unattended delivery/retries, consolidation and mental-model schedules are not enabled: enforceable total-cost policy is unavailable. US$ refers to currency, not location.")
        budgets["cap"] = text("Maximum capture deliveries per turn (0–4)", value: "1")
        heading("Automatic recall limits")
        for (key, label, value) in [("timeout", "Timeout milliseconds (1–5000)", "2000"), ("bytes", "Maximum response bytes (1–32768)", "16384"), ("results", "Maximum results (1–20)", "8"), ("context", "Maximum context characters (100–8000)", "4000")] { budgets[key] = text(label, value: value) }
        healthButton = MemoryActionButton("Check service health / capabilities") { [weak self] in self?.checkHealth() }; addForm(healthButton)
        heading("Explicit service memory")
        let textScroll = NSScrollView(); textScroll.documentView = retainText; textScroll.hasVerticalScroller = true; textScroll.borderType = .bezelBorder
        retainText.isRichText = false; retainText.font = .systemFont(ofSize: 13); retainText.isHorizontallyResizable = false; retainText.isVerticallyResizable = true
        retainText.autoresizingMask = [.width]; retainText.textContainer?.widthTracksTextView = true; retainText.textContainerInset = NSSize(width: 6, height: 6)
        retainText.setAccessibilityLabel(L("Explicit memory text"))
        addForm(textScroll); textScroll.heightAnchor.constraint(equalToConstant: 110).isActive = true
        retainButton.handler = { [weak self] in self?.retain() }; addForm(retainButton)
        query = text("Search / reflection query")
        recallButton = MemoryActionButton("Search this bot's memory…") { [weak self] in self?.queryService(reflect: false) }; addForm(recallButton)
        reflectButton.handler = { [weak self] in self?.queryService(reflect: true) }; addForm(reflectButton)
        note("Native reflect is offered only when negotiated. pgvector and LanceDB do not extract facts or manage mental models. No similarity search is labelled reflection, and no paid synthesis fallback is enabled.")
        heading("Documents and provenance")
        documentID = text("Exact document ID from a result or operation")
        inspectButton = MemoryActionButton("Inspect document and source provenance") { [weak self] in self?.inspect() }; addForm(inspectButton)
        deleteButton = MemoryActionButton("Delete this document…") { [weak self] in self?.deleteMemory(clear: false) }; addForm(deleteButton)
        clearButton = MemoryActionButton("Clear this bot's service bank…") { [weak self] in self?.deleteMemory(clear: true) }; addForm(clearButton)
        note("Pending deletion is not verified erasure. In-flight work can keep cleanup pending; backups and Lance history can retain data. Disabling capture or disconnecting does not erase remote data.")
        heading("Deliveries and operations")
        addForm(operationSummary); row("Core-issued operation", operationMenu)
        operationMenu.target = self; operationMenu.action = #selector(operationChanged)
        loadOperationsButton = MemoryActionButton("Load operations / deletion status") { [weak self] in self?.loadOperations() }; addForm(loadOperationsButton)
        statusButton = MemoryActionButton("Check selected operation") { [weak self] in self?.operation("status") }
        cancelOperationButton = MemoryActionButton("Cancel selected operation…") { [weak self] in self?.operation("cancel") }
        retryButton = MemoryActionButton("Retry selected delivery…") { [weak self] in self?.operation("retry") }
        addForm(statusButton); addForm(cancelOperationButton); addForm(retryButton)
        heading("Advanced capabilities")
        advancedButton = MemoryActionButton("Open negotiated provider controls…") { [weak self] in self?.advanced() }; addForm(advancedButton)
        note("Hindsight profile/config, directives, mental models/history, observations and curation stay distinct from OpenViking sessions/resources/tasks. Only verified capabilities become actionable. Advanced operations can incur downstream costs and require explicit confirmation.")
        heading("Assigned Runner setup")
        setupMenu = popup("Setup action", choices: ["pgvector initialization preview", "LanceDB local directory binding", "LanceDB plaintext export", "LanceDB plaintext import"])
        setupButton = MemoryActionButton("Preview selected setup…") { [weak self] in self?.openSetup() }; addForm(setupButton)
        setButtons(confirm: L("Save preferences")); updateCapabilities()
        confirmButton.isEnabled = false
    }
    override func viewDidAppear() { super.viewDidAppear(); if saved == nil { reloadPreferences() } }
    func reloadPreferences() {
        run { [self] in
            let preserveDraft = editableBaseline != nil
            authorityRecovered = false; health = nil
            let selectedID = connectionMenu.selectedItem?.representedObject as? String
            let list = try await service.listConnections(), preferences = try await service.getPreferences(botID: botID)
            if preserveDraft, let selectedID, !list.connections.contains(where: { $0.id == selectedID }) { throw MemoryUIError.staleTarget }
            inventory = list; saved = preferences
            connectionMenu.removeAllItems(); connectionMenu.addItem(withTitle: L("Disabled — local memory only"))
            for connection in list.connections {
                connectionMenu.addItem(withTitle: "\(connection.name) · \(connection.backend.rawValue)"); connectionMenu.lastItem?.representedObject = connection.id
            }
            let connectionID = preserveDraft ? selectedID : preferences.connectionID
            if let item = connectionMenu.itemArray.first(where: { $0.representedObject as? String == connectionID }) { connectionMenu.select(item) }
            if !preserveDraft {
                editableBaseline = preferences.editable
                draft = .init(botID: botID, saved: preferences.editable)
                autoRecall.state = preferences.autoRecall ? .on : .off; capture.state = preferences.captureConversation ? .on : .off; groupCapture.state = preferences.captureGroupText ? .on : .off
                budgets["cap"]?.stringValue = String(preferences.maxCaptureDeliveriesPerTurn)
                budgets["timeout"]?.stringValue = String(preferences.recallBudget.timeoutMS); budgets["bytes"]?.stringValue = String(preferences.recallBudget.maxBytes)
                budgets["results"]?.stringValue = String(preferences.recallBudget.maxResults); budgets["context"]?.stringValue = String(preferences.recallBudget.maxContextChars)
            }
            health = nil
            authorityRecovered = true
        }
    }
    @objc private func connectionChanged() { draft.connectionID = connectionMenu.selectedItem?.representedObject as? String; health = nil; updateCapabilities() }
    @objc private func operationChanged() { updateCapabilities() }
    private var currentConnection: MemoryServiceConnectionView? { inventory?.connections.first { $0.id == saved?.connectionID } }
    private var selectedOperation: MemoryServiceOperation? { operations?.operations.first { $0.id == operationMenu.selectedItem?.representedObject as? String } }
    func updateCapabilities() {
        if !authorityRecovered { health = nil; operations = nil; operationMenu.removeAllItems() }
        healthButton?.isEnabled = authorityRecovered; loadOperationsButton?.isEnabled = authorityRecovered
        let selected = inventory?.connections.first { $0.id == connectionMenu.selectedItem?.representedObject as? String }
        if selected?.availability == .blocked { health = nil }
        let caps = health?.capabilities ?? .init()
        retainButton.isEnabled = caps.supports(.retain); recallButton?.isEnabled = caps.supports(.recall)
        reflectButton.isEnabled = caps.supports(.reflect); reflectButton.isHidden = !caps.supports(.reflect)
        inspectButton?.isEnabled = caps.supports(.inspect); deleteButton?.isEnabled = caps.supports(.deleteDocument)
        clearButton?.isEnabled = caps.supports(.clear)
        advancedButton?.isEnabled = MemoryAdvancedViewController.features(backend: currentConnection?.backend, capabilities: caps).isEmpty == false
        advancedButton?.isHidden = advancedButton?.isEnabled != true
        statusButton?.isEnabled = selectedOperation != nil && caps.supports(.operationStatus)
        cancelOperationButton?.isEnabled = selectedOperation != nil && caps.supports(.cancelOperation)
        retryButton?.isEnabled = selectedOperation.map { caps.supports(.retain) && ($0.state == .failed || ($0.state == .deliveryUnknown && caps.idempotentRetain)) } ?? false
        setupButton?.isEnabled = authorityRecovered && currentConnection?.availability != .blocked && (currentConnection?.backend == .pgvector || currentConnection?.backend == .lanceDB)
        if selected?.availability == .blocked {
            statusLabel.stringValue = L("Unavailable: %@", selected?.reason ?? MemoryUIError.unavailable.rawValue)
        } else if let health {
            statusLabel.stringValue = L("Service: %@. %@", health.status.rawValue, health.deletionPending ? L("Deletion pending — not verified erasure.") : L("No pending deletion reported."))
            if let reason = health.reason { statusLabel.stringValue += " · " + reason }
        } else {
            statusLabel.stringValue = L("No negotiated service capabilities. Local MEMORY.md remains usable.")
        }
    }
    override func didFinishWork() { updateCapabilities(); confirmButton.isEnabled = authorityRecovered && saved != nil }
    private func readDraft() throws {
        guard saved != nil else { throw MemoryUIError.unavailable }
        draft.connectionID = connectionMenu.selectedItem?.representedObject as? String
        draft.autoRecall = autoRecall.state == .on; draft.captureConversation = capture.state == .on; draft.captureGroupText = groupCapture.state == .on
        guard let cap = Int(budgets["cap"]!.stringValue), let timeout = UInt64(budgets["timeout"]!.stringValue), let bytes = Int(budgets["bytes"]!.stringValue), let results = Int(budgets["results"]!.stringValue), let context = Int(budgets["context"]!.stringValue) else { throw MemoryUIError.invalidInput }
        draft.maxCaptureDeliveriesPerTurn = cap; draft.recallBudget = .init(timeoutMS: timeout, maxBytes: bytes, maxResults: results, maxContextChars: context)
        draft.unattendedCapture = false
        if let selected = inventory?.connections.first(where: { $0.id == draft.connectionID }), selected.availability == .blocked {
            throw selected.reason == MemoryUIError.lanceCloudTransportUnavailable.rawValue ? MemoryUIError.lanceCloudTransportUnavailable : MemoryUIError.unavailable
        }
    }
    override func confirmTapped() {
        guard !busy, authorityRecovered else { return }
        do {
            try readDraft()
            run { [self] in
                // A declined group prompt can leave the exact plaintext approval intact.
                // Resolve whichever approval is currently missing; never send until valid.
                for _ in 0..<3 {
                    do { _ = try draft.request(); break }
                    catch MemoryServiceDraftError.plaintextConsentRequired {
                        guard await confirm("Allow remote memory plaintext and costs?", details: consentDescription) else { return }
                        draft.approveRemotePlaintext()
                    }
                    catch MemoryServiceDraftError.groupConsentRequired {
                        guard await confirm("Also capture future group text?", details: consentDescription + L("\nGroup conversation text can include other participants. It does not grant access to another bot's bank.")) else { return }
                        draft.approveGroupCapture()
                    }
                }
                let data = try await service.savePreferences(draft)
                saved = try JSONDecoder().decode(MemoryServicePreferencesView.self, from: data)
                editableBaseline = saved!.editable
                draft = .init(botID: botID, saved: saved!.editable); health = nil
                onSaved?(); dismiss(nil)
            }
        } catch { showError(error) }
    }
    private var consentDescription: String {
        L("Bot %@; connection %@. Automatic recall: %@. Future conversation capture: %@. Group capture: %@. Capture cap: %d per admitted turn. The endpoint and downstream models receive plaintext and may charge you; no historical backfill, unattended retries or scheduled refresh is authorized.", botID, draft.connectionID ?? "disabled", String(draft.autoRecall), String(draft.captureConversation), String(draft.captureGroupText), draft.maxCaptureDeliveriesPerTurn)
    }
    private func checkHealth() {
        guard authorityRecovered else { return }
        run { [self] in
            guard saved?.connectionID != nil else { throw MemoryUIError.invalidInput }
            guard connectionMenu.selectedItem?.representedObject as? String == saved?.connectionID else { throw MemoryUIError.staleTarget }
            if currentConnection?.availability == .blocked {
                throw currentConnection?.reason == MemoryUIError.lanceCloudTransportUnavailable.rawValue ? MemoryUIError.lanceCloudTransportUnavailable : MemoryUIError.unavailable
            }
            health = try await service.health(botID: botID)
        }
    }
    private func retain() {
        guard health?.capabilities.supports(.retain) == true else { return }
        let text = retainText.string
        guard !text.isEmpty, text.utf8.count <= 32768 else { showError(MemoryUIError.invalidInput); return }
        if retainedDraft?.text != text { retainedDraft = (text, UUID().uuidString.lowercased()) }
        let requestID = retainedDraft!.requestID
        run { [self] in
            guard await confirm("Save explicit service memory?", details: L("Bot %@; connection %@. Send this text to the service and its downstream models; this may incur costs. Stable request IDs do not guarantee exactly-once billing.\n\n%@", botID, currentConnection?.name ?? "", text)) else { return }
            let data = try await service.request("memory.service.retain", ["bot_id": botID, "text": text, "request_id": requestID])
            let operation = try JSONDecoder().decode(MemoryServiceOperation.self, from: data)
            operationSummary.stringValue = Self.describe(operation)
            if operation.state == .completed { retainText.string = ""; retainedDraft = nil }
            output(Self.describe(operation))
        }
    }
    static func describe(_ operation: MemoryServiceOperation) -> String {
        "\(operation.state == .completed ? L("Stored") : L("Not confirmed stored")): \(operation.state.rawValue)\nDelivery: \(operation.id)\nDocument: \(operation.documentID)\nService operation: \(operation.operationID ?? "none")\nError code: \(operation.errorCode ?? "none")"
    }
    private func queryService(reflect: Bool) {
        let action: MemoryServiceAction = reflect ? .reflect : .recall
        guard health?.capabilities.supports(action) == true else { return }
        let text = query.stringValue
        guard !text.isEmpty, text.count <= 4096 else { showError(MemoryUIError.invalidInput); return }
        run { [self] in
            guard await confirm(reflect ? "Run native Hindsight reflect?" : "Search service memory?", details: L("Bot %@; connection %@. The query is plaintext. %@\n\n%@", botID, currentConnection?.name ?? "", reflect ? L("Native reflect uses the service's models and may incur downstream costs. No schedule or background refresh is enabled.") : L("Search can invoke paid embeddings or reranking."), text)) else { return }
            output(try MemoryServiceResultFormatting.render(await service.request("memory.service.\(action.rawValue)", ["bot_id": botID, "query": text])))
        }
    }
    private func inspect() {
        guard health?.capabilities.supports(.inspect) == true, !documentID.stringValue.isEmpty else { return }
        let id = documentID.stringValue
        run { [self] in output(try MemoryServiceResultFormatting.render(await service.request("memory.service.inspect", ["bot_id": botID, "document_id": id]))) }
    }
    private func loadOperations() {
        guard authorityRecovered else { return }
        run { [self] in
            operations = try await service.operations(botID: botID)
            operationMenu.removeAllItems()
            for operation in operations!.operations {
                operationMenu.addItem(withTitle: "\(operation.state.rawValue) · \(operation.documentID) · \(operation.id)"); operationMenu.lastItem?.representedObject = operation.id
            }
            operationSummary.stringValue = operations!.operations.map(Self.describe).joined(separator: "\n\n")
            if operations!.operations.isEmpty { operationSummary.stringValue = L("No durable deliveries reported.") }
            if operations?.deletion?.pending == true { operationSummary.stringValue += L("\nDeletion remains pending — not verified erasure.") }
        }
    }
    private func operation(_ action: String) {
        guard authorityRecovered, let operation = selectedOperation else { return }
        run { [self] in
            if action != "status" {
                guard await confirm(action == "retry" ? "Retry this delivery?" : "Cancel this operation?", details: Self.describe(operation) + L("\nRetry may incur another service charge; cancellation is not verified deletion.")) else { return }
            }
            let reply = try await service.request("memory.operations.\(action)", ["bot_id": botID, "id": operation.id])
            output(Self.describe(try JSONDecoder().decode(MemoryServiceOperation.self, from: reply)))
        }
    }
    private func deleteMemory(clear: Bool) {
        let document = clear ? nil : documentID.stringValue
        guard authorityRecovered, clear || document?.isEmpty == false else { return }
        run { [self] in
            let preferences = try await service.getPreferences(botID: botID), list = try await service.listConnections()
            guard let connection = list.connections.first(where: { $0.id == preferences.connectionID }) else { throw MemoryUIError.staleTarget }
            guard await confirm(clear ? "Clear this bot's service bank?" : "Delete this service document?", details: L("Bot %@; connection %@ (%@), revision %llu · %@; deletion epoch %llu. Target: %@. This fences queued writes; pending cleanup does not mean erasure. Service backups and Lance history may retain data.", botID, connection.name, connection.id, connection.revision.counter, connection.revision.deviceID, preferences.deletionEpoch, document ?? "whole bot bank")) else { return }
            var parameters: [String: Any] = ["bot_id": botID, "confirm": true, "connection_revision": try JSONSerialization.jsonObject(with: JSONEncoder().encode(connection.revision)), "deletion_epoch": preferences.deletionEpoch]
            if let document { parameters["document_id"] = document }
            authorityRecovered = false
            let reply = try await service.request("memory.service.delete", parameters)
            struct Deletion: Decodable { let pending: Bool, deletion_epoch: UInt64 }
            let deletion = try JSONDecoder().decode(Deletion.self, from: reply)
            let message = deletion.pending ? L("Deletion pending. Remote quiescence/erasure is not verified.") : L("Live service deletion completed. Backups/history may retain data.")
            operationSummary.stringValue = L("Deletion epoch %llu. %@", deletion.deletion_epoch, message)
            output(message)
        }
    }
    private func advanced() {
        guard authorityRecovered, let connection = currentConnection, let health else { return }
        presentAsSheet(MemoryAdvancedViewController(service: service, botID: botID, backend: connection.backend, capabilities: health.capabilities))
    }
    private func openSetup() {
        guard authorityRecovered, let connection = currentConnection else { return }
        guard connection.availability != .blocked else { showError(MemoryUIError.lanceCloudTransportUnavailable); return }
        let actions: [MemoryBotSetupAction] = [.pgvector, .lanceBinding, .lanceExport, .lanceImport]
        let action = actions[setupMenu.indexOfSelectedItem]
        guard (action == .pgvector && connection.backend == .pgvector) || (action != .pgvector && connection.backend == .lanceDB) else { showError(MemoryUIError.unsupported); return }
        presentAsSheet(MemoryBotSetupViewController(service: service, setup: setup, botID: botID, connectionID: connection.id, action: action, currentRunner: currentRunner))
    }
}
