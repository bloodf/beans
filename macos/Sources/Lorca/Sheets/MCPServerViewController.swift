import AppKit

/// Edits one server on the selected Runner. Config is fetched only when this sheet opens.
final class MCPServerViewController: SheetViewController {
    private let store = AppStore.shared
    private let runner: Device
    private let originalName: String?
    private let nameField = NSTextField()
    private let configField = NSTextView()
    private let state = SectionView(title: L("Status"))
    private let tools = SectionView(title: L("Tools"))
    private var toolsScroll: NSScrollView?
    private let enabled = NSButton(checkboxWithTitle: L("Enabled"), target: nil, action: nil)
    private let actions = Build.stack([], orientation: .horizontal, spacing: 8)
    private let saveButton = NSButton()
    private var server: MCPServer?
    private var originalConfig: MCPJSON = .object([:])
    private var hasLoadedConfig = false
    private var loading = 0
    var onChange: (() -> Void)?

    init(name: String?, runner: Device, config: [String: MCPJSON]? = nil) {
        originalName = name
        self.runner = runner
        originalConfig = .object(config ?? [:])
        super.init(title: name ?? L("Add MCP Server"), subtitle: L("Installed on %@.", runner.name), width: 560)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        nameField.placeholderString = L("Server name")
        nameField.stringValue = originalName ?? ""
        nameField.setAccessibilityLabel(L("Server name"))
        let nameRow = FieldRow(key: L("Name"), field: nameField)
        contentStack.addArrangedSubview(nameRow)
        nameRow.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true

        let configLabel = Build.label(L("Configuration (JSON)"), font: Theme.Font.caption)
        contentStack.addArrangedSubview(configLabel)
        let scroll = NSScrollView()
        scroll.hasVerticalScroller = true
        scroll.borderType = .bezelBorder
        configField.minSize = NSSize(width: 0, height: 180)
        configField.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        configField.isVerticallyResizable = true
        configField.isHorizontallyResizable = false
        configField.autoresizingMask = [.width]
        configField.textContainer?.widthTracksTextView = true
        scroll.documentView = configField
        configField.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
        configField.isAutomaticQuoteSubstitutionEnabled = false
        configField.isAutomaticDashSubstitutionEnabled = false
        configField.setAccessibilityLabel(L("Configuration (JSON)"))
        contentStack.addArrangedSubview(scroll)
        NSLayoutConstraint.activate([
            scroll.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            scroll.heightAnchor.constraint(equalToConstant: 180),
        ])
        contentStack.addArrangedSubview(enabled)
        enabled.target = self
        enabled.action = #selector(toggleEnabled)
        enabled.isHidden = originalName == nil
        contentStack.addArrangedSubview(state)
        state.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        state.isHidden = originalName == nil
        let toolScroll = NSScrollView()
        toolScroll.hasVerticalScroller = true
        toolScroll.documentView = tools
        tools.widthAnchor.constraint(equalTo: toolScroll.contentView.widthAnchor).isActive = true
        tools.topAnchor.constraint(equalTo: toolScroll.contentView.topAnchor).isActive = true
        contentStack.addArrangedSubview(toolScroll)
        toolScroll.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        toolScroll.heightAnchor.constraint(equalToConstant: 200).isActive = true
        toolScroll.isHidden = true
        toolsScroll = toolScroll
        contentStack.addArrangedSubview(actions)
        actions.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        saveButton.title = L("Save")
        saveButton.target = self
        saveButton.action = #selector(save)
        saveButton.isEnabled = originalName == nil
        actions.addArrangedSubview(saveButton)
        if originalName != nil {
            for (title, action) in [(L("Reconnect"), #selector(reconnect)), (L("Sign in"), #selector(signIn)), (L("Sign out"), #selector(signOut)), (L("Remove…"), #selector(remove))] {
                let button = NSButton(title: title, target: self, action: action)
                button.bezelStyle = .rounded
                actions.addArrangedSubview(button)
            }
        }
        setButtons(confirm: L("Done"), cancel: nil)
        showConfig(originalConfig)
        if originalName != nil { load() }
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        store.observe(self) { [weak self] event in
            guard let self, self.originalName != nil else { return }
            switch event {
            case .rosterChanged, .snapshotReplaced: self.load()
            default: break
            }
        }
    }

    private func showConfig(_ config: MCPJSON, redacting: Bool = true) {
        let redacted = redacting ? Self.redact(config) : config
        guard let data = try? JSONEncoder().encode(redacted),
              let object = try? JSONSerialization.jsonObject(with: data, options: [.fragmentsAllowed]),
              let pretty = try? JSONSerialization.data(withJSONObject: object, options: [.prettyPrinted, .sortedKeys, .fragmentsAllowed])
        else { return }
        configField.string = String(decoding: pretty, as: UTF8.self)
    }


    private static let masked = "••••••••"
    private static func redact(_ value: MCPJSON, key: String = "", parent: String = "") -> MCPJSON {
        switch value {
        case let .object(object):
            return .object(object.mapValues { $0 }.map { ($0.key, redact($0.value, key: $0.key, parent: key)) }
                .reduce(into: [:]) { $0[$1.0] = $1.1 })
        case let .array(array): return .array(array.map { redact($0, key: key, parent: parent) })
        case let .string(text) where key.lowercased() == "url" && urlHasCredentials(text):
            return .string(masked)
        case .string where parent.lowercased() == "env" || parent.lowercased() == "headers" ||
            ["token", "secret", "password", "authorization", "credential", "api_key", "apikey"].contains(where: { key.lowercased().contains($0) }):
            return .string(masked)
        default: return value
        }
    }

    private static func urlHasCredentials(_ text: String) -> Bool {
        let url = URLComponents(string: text)
        return url?.user != nil || url?.password != nil || url?.query != nil
    }

    private static func restore(_ value: MCPJSON, original: MCPJSON?) -> MCPJSON {
        switch (value, original) {
        case let (.string(text), old?) where text == masked: return old
        case let (.object(object), .object(old)):
            return .object(object.mapValues { $0 }.map { ($0.key, restore($0.value, original: old[$0.key])) }
                .reduce(into: [:]) { $0[$1.0] = $1.1 })
        default: return value
        }
    }

    /// Keep user edits (including removed keys) while rebasing untouched fields onto a live change.
    static func rebaseDraft(_ draft: MCPJSON?, from old: MCPJSON?, to new: MCPJSON?) -> MCPJSON? {
        if draft == old { return new }
        if case let .object(draftValues) = draft, case let .object(oldValues) = old,
           case let .object(newValues) = new {
            var merged: [String: MCPJSON] = [:]
            for key in Set(draftValues.keys).union(oldValues.keys).union(newValues.keys) {
                merged[key] = rebaseDraft(draftValues[key], from: oldValues[key], to: newValues[key])
            }
            return .object(merged)
        }
        return draft
    }

    static func plannedSaves(_ parsed: MCPParsedServers, name: String, previousName: String?, existingNames: Set<String>) throws -> [(String, [String: MCPJSON])] {
        guard !parsed.servers.isEmpty else { throw MCPDraftError.message(L("No MCP servers in that JSON")) }
        guard previousName == nil || parsed.servers.count == 1 else { throw MCPDraftError.message(L("Edit one server at a time")) }
        var planned: [(String, [String: MCPJSON])] = []
        var names = Set<String>()
        for entry in parsed.servers {
            if let problem = entry.problem { throw MCPDraftError.message(problem) }
            let resolved = (parsed.servers.count == 1 && !name.isEmpty ? name : entry.name ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            guard !resolved.isEmpty else { throw MCPDraftError.message(L("Server name is required")) }
            guard !names.contains(resolved), !existingNames.contains(resolved) || resolved == previousName else {
                throw MCPDraftError.message(L("A server named %@ already exists", resolved))
            }
            guard case let .object(config) = entry.config else { throw MCPDraftError.message(L("Configuration must be a JSON object")) }
            names.insert(resolved)
            planned.append((resolved, config))
        }
        return planned
    }

    private enum MCPDraftError: LocalizedError {
        case message(String)
        var errorDescription: String? { if case let .message(text) = self { return text }; return nil }
    }

    private func load() {
        guard let originalName else { return }
        loading += 1
        let generation = loading
        Task { [weak self] in
            guard let self else { return }
            do {
                let response = try await store.mcpServer(originalName, on: runner.id)
                guard loading == generation else { return }
                server = response.server
                saveButton.isEnabled = true
                // Status refreshes must not replace a config draft the user is editing.
                if !hasLoadedConfig {
                    hasLoadedConfig = true
                    originalConfig = response.server.config
                    showConfig(originalConfig)
                }
                render(response.server)
            } catch {
                guard loading == generation else { return }
                state.setRows([KeyValueRow(key: L("State"), value: error.localizedDescription, tint: .systemRed)])
            }
        }
    }

    private func render(_ server: MCPServer) {
        enabled.state = server.enabled ? .on : .off
        var rows: [NSView] = [KeyValueRow(key: L("State"), value: server.problem ?? server.status?.detail ?? server.transport ?? "")]
        if server.signsIn { rows.append(KeyValueRow(key: L("Sign-in"), value: server.signedIn ? L("Signed in") : L("Not signed in"))) }
        state.setRows(rows)
        toolsScroll?.isHidden = server.tools?.isEmpty != false
        tools.setRows((server.tools ?? []).map { tool in
            let row = ActionRow(key: tool.name, value: tool.description ?? "", tint: .secondaryLabelColor, actionTitle: tool.hidden ? L("Show") : L("Hide"))
            row.actionView.setAccessibilityLabel(L("%@ tool %@", tool.hidden ? L("Show") : L("Hide"), tool.name))
            row.onAction = { [weak self] in
                self?.change("mcp.hide_tool", values: ["tool": tool.name, "hidden": !tool.hidden])
            }
            return row
        })
        for button in actions.arrangedSubviews.compactMap({ $0 as? NSButton }) {
            if button.action == #selector(signIn) { button.isHidden = !server.signsIn || server.signedIn }
            if button.action == #selector(signOut) { button.isHidden = !server.signsIn || !server.signedIn }
        }
        fitSheetToContent()
    }


    @objc private func save() {
        let name = nameField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        let draft: String
        let pasted = configField.string
        if let data = pasted.data(using: .utf8), let object = try? JSONDecoder().decode(MCPJSON.self, from: data),
           let restored = try? JSONEncoder().encode(Self.restore(object, original: originalConfig)) {
            draft = String(decoding: restored, as: UTF8.self)
        } else {
            draft = pasted
        }
        saveButton.isEnabled = false
        Task { [weak self] in
            guard let self else { return }
            defer { saveButton.isEnabled = true }
            do {
                let parsed = try await store.parseMCPServers(draft)
                let listing = try await store.mcpServers(on: runner.id)
                if let error = listing.error { throw MCPDraftError.message(error) }
                let saves = try Self.plannedSaves(parsed, name: name, previousName: originalName,
                    existingNames: Set(listing.servers.map(\.name)))
                var added: [String] = []
                do {
                    for (serverName, config) in saves {
                        _ = try await store.saveMCPServer(serverName, previousName: originalName, config: config, on: runner.id)
                        if originalName == nil { added.append(serverName) }
                    }
                } catch {
                    for serverName in added.reversed() {
                        do { try await store.removeMCPServer(serverName, on: runner.id) }
                        catch { onChange?(); throw MCPDraftError.message(L("Import failed; check saved servers before retrying")) }
                    }
                    throw error
                }
                onChange?()
                dismiss(nil)
            } catch { alert(error.localizedDescription) }
        }
    }

    @objc private func toggleEnabled() {
        change("mcp.set_enabled", values: ["enabled": enabled.state == .on])
    }

    @objc private func reconnect() { change("mcp.reconnect") }

    @objc private func signIn() {
        guard let server else { return }
        Task { [weak self] in
            guard let self else { return }
            do { try await store.signInMCPServer(server, on: runner.id); load() }
            catch { alert(error.localizedDescription) }
        }
    }

    @objc private func signOut() { change("mcp.sign_out") }

    private func acceptLiveConfig(_ config: MCPJSON) {
        let old = originalConfig
        originalConfig = config
        guard let data = configField.string.data(using: .utf8),
              let draft = try? JSONDecoder().decode(MCPJSON.self, from: data),
              let merged = Self.rebaseDraft(draft, from: Self.redact(old), to: Self.redact(config)) else { return }
        if merged != draft { showConfig(merged, redacting: false) }
    }

    private func change(_ method: String, values: [String: Any] = [:]) {
        guard let name = server?.name else { return }
        Task { [weak self] in
            guard let self else { return }
            do {
                let response = try await store.changeMCPServer(name, on: runner.id, method: method, values: values)
                server = response.server
                if method == "mcp.set_enabled" || method == "mcp.hide_tool" {
                    acceptLiveConfig(response.server.config)
                }
                render(response.server)
                onChange?()
            } catch { alert(error.localizedDescription); if method == "mcp.set_enabled" { enabled.state = enabled.state == .on ? .off : .on } }
        }
    }

    @objc private func remove() {
        guard let name = originalName, let window = view.window else { return }
        let confirmation = NSAlert()
        confirmation.messageText = L("Remove %@ from %@?", name, runner.name)
        confirmation.informativeText = L("Every bot on %@ loses it, and its keys and sign-ins there are forgotten.", runner.name)
        confirmation.addButton(withTitle: L("Remove"))
        confirmation.addButton(withTitle: L("Cancel"))
        confirmation.alertStyle = .warning
        confirmation.beginSheetModal(for: window) { [weak self] result in
            guard let self, result == .alertFirstButtonReturn else { return }
            Task { [weak self] in
                guard let self else { return }
                do { try await store.removeMCPServer(name, on: runner.id); onChange?(); dismiss(nil) }
                catch { alert(error.localizedDescription) }
            }
        }
    }

    private func alert(_ text: String) {
        let alert = NSAlert()
        alert.messageText = text
        if let window = view.window { alert.beginSheetModal(for: window) }
    }
}
