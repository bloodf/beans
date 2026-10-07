import AppKit

final class MemoryConnectionViewController: MemoryFormViewController {
    let service: MemoryServiceAPI
    private let connectionID: String
    let saved: MemoryServiceConnectionView?
    let embeddings: [MemoryServiceEmbeddingView]
    let bots: [(id: String, name: String)]
    let nameField = NSTextField()
    let backendMenu = NSPopUpButton()
    let endpointMode = NSPopUpButton()
    let endpointField = NSTextField()
    let secret: MemorySecretControl
    private let embeddingMenu = NSPopUpButton()
    private let insecure = NSButton(checkboxWithTitle: L("Allow private HTTP — plaintext transport"), target: nil, action: nil)
    private let replaceOptions = NSButton(checkboxWithTitle: L("Edit backend options"), target: nil, action: nil)
    private let optionsColumn = Build.stack([], spacing: 12)
    private var schemaField: NSTextField?, roleField: NSTextField?
    private var bindings: [MemoryOpenVikingBindingForm] = []
    private let backends: [MemoryServiceBackend] = [.hindsight, .openViking, .pgvector, .lanceDB]
    var backend: MemoryServiceBackend { backends[max(0, backendMenu.indexOfSelectedItem)] }

    init(service: MemoryServiceAPI, saved: MemoryServiceConnectionView?, embeddings: [MemoryServiceEmbeddingView], bots: [(id: String, name: String)]) {
        self.service = service; self.saved = saved; self.embeddings = embeddings; self.bots = bots
        connectionID = saved?.id ?? "memory-\(UUID().uuidString.lowercased())"
        secret = MemorySecretControl(hasSecret: saved?.hasSecret ?? false)
        super.init(title: L(saved == nil ? "New memory connection" : "Edit memory connection"), subtitle: L("Account configuration syncs encrypted. A saved connection is not proof of a ready service."), width: 580)
    }
    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }
    override func loadView() {
        super.loadView()
        row("Name", nameField); nameField.stringValue = saved?.name ?? ""
        backendMenu.addItems(withTitles: ["Hindsight", "OpenViking", "pgvector", L("LanceDB — local setup only")])
        if let saved { backendMenu.selectItem(at: backends.firstIndex(of: saved.backend)!) }
        backendMenu.isEnabled = saved == nil; backendMenu.target = self; backendMenu.action = #selector(backendChanged)
        row("Backend", backendMenu)
        endpointMode.addItems(withTitles: [L("Keep saved endpoint (masked)"), L("Set exact endpoint"), L("Clear endpoint — local LanceDB only")])
        endpointMode.menu?.autoenablesItems = false
        if saved == nil { endpointMode.selectItem(at: 1) }
        row("Endpoint action", endpointMode); row("Endpoint", endpointField)
        endpointMode.target = self; endpointMode.action = #selector(endpointChanged)
        endpointField.placeholderString = L("No embedded credentials, query or fragment")
        row("Service key / PostgreSQL password", secret)
        embeddingMenu.addItem(withTitle: L("Keep existing embedding profile"))
        embeddingMenu.addItem(withTitle: L("No embedding profile"))
        for profile in embeddings {
            embeddingMenu.addItem(withTitle: "\(profile.model) · \(profile.id)")
            embeddingMenu.lastItem?.representedObject = profile.id
        }
        row("Embedding profile (pgvector / LanceDB)", embeddingMenu)
        addForm(insecure)
        replaceOptions.state = saved == nil ? .on : .off
        replaceOptions.target = self; replaceOptions.action = #selector(optionsChanged)
        addForm(replaceOptions); addForm(optionsColumn)
        note("Saved endpoints and private bindings are never read back. Leave endpoint and options unchanged to preserve them. Setting a new endpoint associates the selected secret action atomically; verify that the key belongs to that target.")
        if saved?.availability == .blocked { note(L("Unavailable: %@", saved?.reason ?? MemoryUIError.unavailable.rawValue)) }
        note("Remote memory and API embeddings process plaintext outside the relay's zero-knowledge guarantee. Your service may use paid extraction, embedding, reranking or synthesis models. Disconnection does not erase remote data.")
        setButtons(confirm: L("Save")); rebuildOptions(); endpointChanged()
    }
    @objc private func endpointChanged() { endpointField.isEnabled = endpointMode.indexOfSelectedItem == 1 }
    @objc private func backendChanged() { rebuildOptions() }
    @objc private func optionsChanged() { optionsColumn.isHidden = replaceOptions.state != .on }
    private func optionRow(_ title: String, _ view: NSView) {
        view.translatesAutoresizingMaskIntoConstraints = false; view.setAccessibilityLabel(L(title))
        let stack = Build.stack([Build.label(L(title), font: .systemFont(ofSize: 12)), view], spacing: 5)
        optionsColumn.addArrangedSubview(stack)
        stack.widthAnchor.constraint(equalTo: optionsColumn.widthAnchor).isActive = true
        view.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
    }
    private func rebuildOptions() {
        endpointMode.item(at: 1)?.isEnabled = backend != .lanceDB
        if saved == nil && backend == .lanceDB { endpointMode.selectItem(at: 2) }
        endpointChanged()
        optionsColumn.arrangedSubviews.forEach { optionsColumn.removeArrangedSubview($0); $0.removeFromSuperview() }
        schemaField = nil; roleField = nil; bindings = []
        switch backend {
        case .hindsight:
            optionRow("Hindsight", Build.label(L("Bank scope is derived by core for each bot. No bank selector."), font: .systemFont(ofSize: 12), lines: 0))
        case .pgvector:
            let schema = NSTextField(string: "beans_memory"); schemaField = schema; optionRow("Schema", schema)
            let role = NSTextField(); roleField = role; optionRow("PostgreSQL login role", role)
            optionRow("Initialization", Build.label(L("Verified TLS is required. Save configuration, then preview the actual database target and non-destructive SQL from the bot's Memory Service page."), font: .systemFont(ofSize: 12), lines: 0))
        case .lanceDB:
            optionRow("Local LanceDB", Build.label(L("Use a cleared endpoint and an explicitly approved directory on the assigned Runner. Cloud activation is blocked: lancedb_cloud_transport_unavailable. Saved endpoints remain masked and are not probed here."), font: .systemFont(ofSize: 12), lines: 0))
        case .openViking:
            optionRow("Authorization", Build.label(L("Use an exact bot's USER key or a trusted gateway identity. Shared USER headers do not isolate bots. Omitted bot bindings are preserved; explicitly mark a selected bot's authorization for removal to send null."), font: .systemFont(ofSize: 12), lines: 0))
            let button = MemoryActionButton("Add bot authorization") { [weak self] in self?.addBinding() }
            optionsColumn.addArrangedSubview(button)
            addBinding()
        }
        optionsChanged()
    }
    private func addBinding() {
        let form = MemoryOpenVikingBindingForm(bots: bots)
        bindings.append(form); optionsColumn.addArrangedSubview(form)
        form.widthAnchor.constraint(equalTo: optionsColumn.widthAnchor).isActive = true
        form.onRemove = { [weak self, weak form] in
            guard let self, let form else { return }
            self.bindings.removeAll { $0 === form }; self.optionsColumn.removeArrangedSubview(form); form.removeFromSuperview()
        }
    }
    func edit() throws -> MemoryServiceConnectionEdit {
        guard !nameField.stringValue.isEmpty else { throw MemoryUIError.invalidInput }
        var edit = MemoryServiceConnectionEdit(id: connectionID, backend: backend, name: nameField.stringValue)
        edit.secret = try secret.patch()
        switch endpointMode.indexOfSelectedItem {
        case 0: edit.endpoint = .keep
        case 2:
            guard backend == .lanceDB else { throw MemoryUIError.invalidInput }
            edit.endpoint = .clear
        default:
            guard backend != .lanceDB else { throw MemoryUIError.lanceCloudTransportUnavailable }
            let endpoint = endpointField.stringValue
            let schemes: Set<String>
            switch backend { case .hindsight, .openViking: schemes = ["http", "https"]; case .pgvector: schemes = ["postgres", "postgresql"]; case .lanceDB: schemes = ["db"] }
            try MemoryUIValidation.endpoint(endpoint, schemes: schemes)
            guard !endpoint.hasPrefix("http:") || insecure.state == .on else { throw MemoryUIError.confirmationRequired }
            edit.endpoint = .set(endpoint)
        }
        switch embeddingMenu.indexOfSelectedItem {
        case 0: edit.embeddingProfile = .keep
        case 1: edit.embeddingProfile = .clear
        default: edit.embeddingProfile = .set(embeddingMenu.selectedItem?.representedObject as! String)
        }
        // A masked existing HTTP permission is preserved unless setting an endpoint explicitly.
        if saved == nil || endpointMode.indexOfSelectedItem != 0 { edit.allowInsecureHTTP = insecure.state == .on }
        if replaceOptions.state == .on {
            switch backend {
            case .hindsight: edit.options = .hindsight
            case .pgvector:
                guard let schema = schemaField?.stringValue, !schema.isEmpty, schema.utf8.count <= 63,
                      schema.utf8.allSatisfy({ (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0) || $0 == 95 }),
                      let role = roleField?.stringValue, role.utf8.count <= 63 else { throw MemoryUIError.invalidInput }
                edit.options = .pgvector(schema: schema, role: role.isEmpty ? nil : role)
            case .lanceDB:
                if endpointMode.indexOfSelectedItem == 2 { edit.options = .lanceDB(region: nil) }
            case .openViking:
                var values: [String: MemoryServiceOpenVikingBindingPatch] = [:]
                for form in bindings {
                    let (id, value) = try form.binding()
                    guard values[id] == nil else { throw MemoryUIError.invalidInput }; values[id] = value
                }
                edit.options = .openViking(bindings: values)
            }
        }
        return edit
    }
    override func confirmTapped() {
        guard !busy else { return }
        do {
            let edit = try edit()
            let endpoint: String
            switch edit.endpoint {
            case .keep: endpoint = L("Keep saved endpoint (masked)")
            case let .set(value): endpoint = value
            case .clear: endpoint = L("No endpoint — local LanceDB")
            }
            run { [self] in
                if insecure.state == .on && endpointMode.indexOfSelectedItem == 1 {
                    guard await confirm("Allow insecure memory transport?", details: L("The exact endpoint %@ receives plaintext over HTTP, including authentication and memory content.", endpointField.stringValue)) else { return }
                }
                if saved != nil && (endpointMode.indexOfSelectedItem != 0 || replaceOptions.state == .on) {
                    guard await confirm("Change memory connection target/options?", details: L("Connection %@ (%@). Exact target: %@. Secret action: %@. Verify this secret belongs to this target. OpenViking preserves omitted bot bindings; only explicitly marked removals delete authorization. Existing remote data is not moved or erased.", edit.name, edit.id, endpoint, String(describing: edit.secret))) else { return }
                }
                try await service.saved(service.send(.setConnection(edit)))
                finishSave()
            }
        } catch { showError(error) }
    }
}

private final class MemoryOpenVikingBindingForm: NSStackView {
    let bot = NSPopUpButton(), mode = NSPopUpButton(), account = NSTextField(), user = NSTextField()
    let secret = MemorySecretControl(hasSecret: true)
    let removeAuthorization = NSButton(checkboxWithTitle: L("Remove this selected bot's stored authorization"), target: nil, action: nil)
    var onRemove: (() -> Void)?
    init(bots: [(id: String, name: String)]) {
        super.init(frame: .zero); orientation = .vertical; alignment = .leading; spacing = 6
        translatesAutoresizingMaskIntoConstraints = false
        bot.addItem(withTitle: L("Choose exact bot"))
        for value in bots { bot.addItem(withTitle: "\(value.name) · \(value.id)"); bot.lastItem?.representedObject = value.id }
        mode.addItems(withTitles: [L("Bot USER key"), L("Trusted gateway")])
        for (label, view) in [("Bot", bot as NSView), ("Authorization mode", mode), ("Account identity", account), ("User identity", user), ("Bot authorization secret", secret)] {
            let stack = Build.stack([Build.label(L(label), font: .systemFont(ofSize: 12)), view], spacing: 4)
            view.translatesAutoresizingMaskIntoConstraints = false; view.setAccessibilityLabel(L(label))
            addArrangedSubview(stack); stack.widthAnchor.constraint(equalTo: widthAnchor).isActive = true
            view.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
        }
        addArrangedSubview(removeAuthorization)
        addArrangedSubview(MemoryActionButton("Remove binding from draft") { [weak self] in self?.onRemove?() })
    }
    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }
    func binding() throws -> (String, MemoryServiceOpenVikingBindingPatch) {
        guard let id = bot.selectedItem?.representedObject as? String else { throw MemoryUIError.invalidInput }
        if removeAuthorization.state == .on { return (id, .remove) }
        guard !account.stringValue.isEmpty, !user.stringValue.isEmpty else { throw MemoryUIError.invalidInput }
        return (id, .set(.init(mode: mode.indexOfSelectedItem == 0 ? .userKey : .trustedGateway, accountID: account.stringValue, userID: user.stringValue, secret: try secret.patch())))
    }
}
