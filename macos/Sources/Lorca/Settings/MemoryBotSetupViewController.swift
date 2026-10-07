import AppKit

final class MemoryBotSetupViewController: MemoryFormViewController {
    private let service: MemoryServiceAPI, setup: MemorySetupAPI
    private let botID: String, connectionID: String
    private let currentRunner: () -> String?
    private let action: MemoryBotSetupAction
    private var path: NSTextField?, create: NSButton?
    private var approved = NSButton(checkboxWithTitle: L("I approve this exact target and previewed changes"), target: nil, action: nil)
    private let previewText = NSTextView()
    private var apply: (() async throws -> MemoryBotSetupResult)?
    private var previewButton: MemoryActionButton!
    init(service: MemoryServiceAPI, setup: MemorySetupAPI, botID: String, connectionID: String, action: MemoryBotSetupAction, currentRunner: @escaping () -> String?) {
        self.service = service; self.setup = setup; self.botID = botID; self.connectionID = connectionID; self.action = action; self.currentRunner = currentRunner
        super.init(title: L("Memory setup approval"), subtitle: L("A preview is not permission to apply. Tokens are exact-target, expiring and single-use, even on failure."), width: 640)
    }
    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }
    override func loadView() {
        super.loadView()
        note("Bot: \(botID). Setup runs on this bot's assigned Runner. It never replaces existing schemas or silently synchronizes local database files.")
        switch action {
        case .pgvector: heading("pgvector initialization"); note("Preview reads the actual connected database, role, session, schema readiness and complete SQL. Apply can create the previewed extension/schema/tables only after confirmation.")
        case .lanceBinding:
            heading("LanceDB local binding"); path = text("Exact absolute directory on assigned Runner")
            create = toggle("Allow creation of the exact previewed table")
        case .lanceExport, .lanceImport:
            heading(action == .lanceExport ? "Export LanceDB plaintext" : "Import LanceDB plaintext")
            path = text(action == .lanceExport ? "Exact new destination file on assigned Runner" : "Exact source file on assigned Runner")
            note("Transfer is plaintext, scoped to this account and bot. The preview binds document count, checksum and full vector-space fingerprint/generation. Export never overwrites a file. A different Runner needs explicit transfer; account sync does not move database files.")
        }
        previewButton = MemoryActionButton("Obtain exact preview") { [weak self] in self?.preview() }
        addForm(previewButton)
        let scroll = NSScrollView(); scroll.documentView = previewText
        scroll.hasVerticalScroller = true; scroll.borderType = .bezelBorder
        previewText.isEditable = false; previewText.isRichText = false
        previewText.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
        previewText.isVerticallyResizable = true; previewText.isHorizontallyResizable = false
        previewText.autoresizingMask = [.width]; previewText.textContainer?.widthTracksTextView = true
        previewText.textContainerInset = NSSize(width: 8, height: 8)
        addForm(scroll); scroll.heightAnchor.constraint(equalToConstant: 210).isActive = true
        addForm(approved); approved.target = self; approved.action = #selector(approvalChanged)
        setButtons(confirm: L("Apply exact preview")); confirmButton.isEnabled = false
    }
    @objc private func approvalChanged() { confirmButton.isEnabled = apply != nil && approved.state == .on }
    private func target() async throws -> MemoryBotSetupTarget {
        guard let runner = currentRunner() else { throw MemoryUIError.staleTarget }
        let preferences = try await service.getPreferences(botID: botID)
        guard preferences.connectionID == connectionID else { throw MemoryUIError.staleTarget }
        let list = try await service.listConnections()
        guard let connection = list.connections.first(where: { $0.id == connectionID }) else { throw MemoryUIError.staleTarget }
        if connection.availability == .blocked {
            throw connection.reason == MemoryUIError.lanceCloudTransportUnavailable.rawValue ? MemoryUIError.lanceCloudTransportUnavailable : MemoryUIError.unavailable
        }
        return .init(runnerID: runner, botID: botID, connectionRevision: connection.revision)
    }
    func preview() {
        guard !busy else { return }
        if apply != nil {
            apply = nil; approved.state = .off; previewText.string = ""
            didFinishWork()
            return
        }
        apply = nil; approved.state = .off; confirmButton.isEnabled = false
        let exactPath = path?.stringValue ?? "", mayCreate = create?.state == .on
        run { [self] in
            if action != .pgvector { guard exactPath.hasPrefix("/"), !exactPath.contains("\0") else { throw MemoryUIError.invalidInput } }
            let current = try await target()
            switch action {
            case .pgvector:
                let preview = try await setup.previewBot(.pgvectorPreview(botID: botID), as: MemoryPgvectorDetails.self)
                guard preview.target == current else { throw MemorySetupError.approvalStale }
                let approval = try MemoryBotSetupApproval(preview: preview, expected: action)
                previewText.string = MemoryPreviewFormatting.pgvector(preview)
                apply = { [self] in try await setup.applyBot(approval, confirm: true, current: target()) }
            case .lanceBinding:
                let preview = try await setup.previewBot(.lanceBindingPreview(botID: botID, directory: exactPath, create: mayCreate), as: MemoryLanceBindingDetails.self)
                guard preview.target == current else { throw MemorySetupError.approvalStale }
                let approval = try MemoryBotSetupApproval(preview: preview, expected: action)
                previewText.string = "Runner: \(preview.runnerID)\nBot: \(preview.botID)\nDirectory: \(preview.details.directory)\nCreate: \(preview.details.create)\nTable: \(preview.details.table)\nNamespace: \(preview.details.namespace)\nExpires: \(Date(timeIntervalSince1970: preview.expiresAt))"
                apply = { [self] in try await setup.applyBot(approval, confirm: true, current: target()) }
            case .lanceExport, .lanceImport:
                let preview = try await setup.previewBot(.lanceTransferPreview(action: action, botID: botID, path: exactPath), as: MemoryLanceTransferDetails.self)
                guard preview.target == current else { throw MemorySetupError.approvalStale }
                let approval = try MemoryBotSetupApproval(preview: preview, expected: action)
                let details = preview.details
                previewText.string = "Runner: \(preview.runnerID)\nBot: \(preview.botID)\nPath: \(details.path)\nNamespace: \(details.namespace)\nDocuments: \(details.documents)\nSHA-256: \(details.sha256)\nFingerprint: \(details.space.fingerprint)\nDimensions: \(details.space.dimensions)\nDistance: \(details.space.distance.rawValue)\nGeneration: \(details.space.generation)\nExpires: \(Date(timeIntervalSince1970: preview.expiresAt))"
                apply = { [self] in try await setup.applyBot(approval, confirm: true, current: target()) }
            }
            // Freeze input after preview. Another preview is the only way to change its target.
            path?.isEnabled = false; create?.isEnabled = false
        }
    }
    override func confirmTapped() {
        guard !busy, approved.state == .on, let apply else { showError(MemoryUIError.confirmationRequired); return }
        self.apply = nil; confirmButton.isEnabled = false; approved.state = .off
        run { [self] in
            guard await confirm("Apply the displayed exact preview?", details: previewText.string) else { return }
            let result = try await apply()
            switch result {
            case .initialized: output(L("The previewed schema was initialized and verified."))
            case .bindingReady: output(L("The approved local LanceDB binding is ready on this Runner."))
            case let .exported(bytes): output(L("Export complete: %d plaintext bytes. Keep the file private.", Int(bytes)))
            case .imported: output(L("The approved plaintext import completed."))
            }
            onSaved?()
        }
    }
    override func didFinishWork() {
        path?.isEnabled = apply == nil; create?.isEnabled = apply == nil
        previewButton.title = L(apply == nil ? "Obtain exact preview" : "Discard preview / edit target")
        confirmButton.isEnabled = apply != nil && approved.state == .on
    }
}

enum MemoryPreviewFormatting {
    static func pgvector(_ preview: MemoryBotSetupPreview<MemoryPgvectorDetails>) -> String {
        let details = preview.details, target = details.target, readiness = details.readiness
        return """
        Runner: \(preview.runnerID)
        Bot: \(preview.botID)
        Database: \(target.database) (OID \(target.databaseOID))
        Role: \(target.role)
        Server: \(target.serverAddress ?? "local"):\(target.serverPort.map(String.init) ?? "local")
        Version: \(target.serverVersion)
        Session PID: \(target.sessionPID)
        Schema: \(details.schema)
        Ready: \(readiness.ready)
        Extension: \(readiness.extensionVersion ?? "absent")
        Extension schema: \(readiness.extensionSchema ?? "absent")
        Schema exists: \(readiness.schemaExists)
        Can create schema: \(readiness.canCreateSchema)
        Can create tables: \(readiness.canCreateTables)
        Can install extension: \(readiness.canInstallExtension)
        Expires: \(Date(timeIntervalSince1970: preview.expiresAt))

        Exact SQL, indexes and permissions:
        \(details.sql)
        """
    }
}
