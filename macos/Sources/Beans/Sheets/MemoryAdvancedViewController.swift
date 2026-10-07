import AppKit

final class MemoryAdvancedViewController: MemoryFormViewController {
    private let service: MemoryServiceAPI, botID: String, backend: MemoryServiceBackend
    private let capabilities: MemoryServiceCapabilities
    private let featureMenu = NSPopUpButton(), actionMenu = NSPopUpButton()
    private let inputColumn = Build.stack([], spacing: 12)
    private var supported: [MemoryServiceAction] = []
    private(set) var fields: [String: NSTextField] = [:]
    private var choices: [String: NSPopUpButton] = [:]
    private var flags: [String: NSButton] = [:]
    private var definitions: [Field] = []
    private struct Field {
        enum Kind { case text, integer, tags, boolean, role, budget }
        let key: String, label: String, kind: Kind, required: Bool
        init(_ key: String, _ label: String, _ kind: Kind = .text, required: Bool = false) { self.key = key; self.label = label; self.kind = kind; self.required = required }
    }
    init(service: MemoryServiceAPI, botID: String, backend: MemoryServiceBackend, capabilities: MemoryServiceCapabilities) {
        self.service = service; self.botID = botID; self.backend = backend; self.capabilities = capabilities
        super.init(title: L("Advanced memory controls"), subtitle: L("Typed, bot-bound actions for negotiated capabilities only. No bank, account, tenant, URI, table or credential selector."), width: 620)
    }
    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }
    static func features(backend: MemoryServiceBackend?, capabilities: MemoryServiceCapabilities) -> [MemoryServiceAction] {
        let features: [MemoryServiceAction]
        switch backend {
        case .hindsight: features = [.bankProfile, .bankConfig, .directives, .mentalModels, .mentalModelHistory, .observations, .memoryEdit, .memoryInvalidate, .memoryRestore, .documents, .tasks]
        case .openViking: features = [.sessions, .resources, .tasks]
        case .pgvector, .lanceDB: features = [.memoryEdit]
        case nil: features = []
        }
        guard let backend else { return [] }
        return features.filter { feature in
            capabilities.supports(feature) && knownActions(for: feature, backend: backend).contains { capabilities.supports(feature, verb: $0) }
        }
    }
    override func loadView() {
        super.loadView()
        supported = Self.features(backend: backend, capabilities: capabilities)
        note("Bot: \(botID). \(backend.rawValue). Responses remain untrusted historical data. Mutating and model-backed actions require an exact confirmation; reads run only when you press Run.")
        note("Mental-model auto-refresh after consolidation and cron schedules stay disabled. No routine/background-spending authorization or enforceable total-cost evidence is available. Explicit create/refresh/commit can incur downstream charges; token limits are not dollar caps.")
        row("Negotiated feature", featureMenu); row("Action", actionMenu)
        featureMenu.addItems(withTitles: supported.map { $0.rawValue.replacingOccurrences(of: "_", with: " ") })
        featureMenu.target = self; featureMenu.action = #selector(featureChanged)
        actionMenu.target = self; actionMenu.action = #selector(actionChanged)
        addForm(inputColumn)
        setButtons(confirm: L("Run selected action")); featureChanged()
        confirmButton.isEnabled = !supported.isEmpty
    }
    private var feature: MemoryServiceAction? { supported.indices.contains(featureMenu.indexOfSelectedItem) ? supported[featureMenu.indexOfSelectedItem] : nil }
    private var action: String { actionMenu.selectedItem?.representedObject as? String ?? "" }
    private func actions(for feature: MemoryServiceAction) -> [String] {
        Self.knownActions(for: feature, backend: backend).filter { capabilities.supports(feature, verb: $0) }
    }
    private static func knownActions(for feature: MemoryServiceAction, backend: MemoryServiceBackend) -> [String] {
        if backend == .pgvector || backend == .lanceDB { return feature == .memoryEdit ? ["edit"] : [] }
        switch feature {
        case .bankProfile, .bankConfig: return ["get", "update", "reset"]
        case .directives: return ["list", "get", "create", "update", "delete"]
        case .mentalModels: return ["list", "get", "create", "update", "refresh", "history", "delete"]
        case .mentalModelHistory: return ["list", "get"]
        case .observations: return ["list", "scopes", "clear", "clear_derived"]
        case .memoryEdit, .memoryInvalidate, .memoryRestore: return ["get", "history", "update"]
        case .documents: return ["list", "get", "chunks"]
        case .sessions: return ["list", "create", "get", "add_message", "commit", "delete"]
        case .resources: return ["list", "add", "read", "delete"]
        case .tasks: return backend == .hindsight ? ["list", "get", "cancel", "delete_record"] : ["list", "get", "cancel"]
        default: return []
        }
    }
    func select(feature: MemoryServiceAction, action: String) {
        guard let index = supported.firstIndex(of: feature) else { return }
        featureMenu.selectItem(at: index); featureChanged()
        if let item = actionMenu.itemArray.first(where: { $0.representedObject as? String == action }) { actionMenu.select(item); actionChanged() }
    }
    @objc private func featureChanged() {
        actionMenu.removeAllItems()
        if let feature {
            for action in actions(for: feature) {
                actionMenu.addItem(withTitle: L(action.replacingOccurrences(of: "_", with: " ")))
                actionMenu.lastItem?.representedObject = action
            }
        }
        actionChanged()
    }
    @objc private func actionChanged() {
        inputColumn.arrangedSubviews.forEach { inputColumn.removeArrangedSubview($0); $0.removeFromSuperview() }
        fields = [:]; choices = [:]; flags = [:]
        definitions = fieldDefinitions()
        for definition in definitions {
            let control: NSView
            switch definition.kind {
            case .boolean:
                let button = NSButton(checkboxWithTitle: L(definition.label), target: nil, action: nil)
                button.allowsMixedState = true
                button.state = action == "create" ? .on : .mixed
                flags[definition.key] = button; control = button
            case .role, .budget:
                let popup = NSPopUpButton()
                let values = definition.kind == .role ? ["user", "assistant"] : [L("Leave unchanged"), "low", "mid", "high"]
                popup.addItems(withTitles: values); choices[definition.key] = popup; control = popup
            default:
                let field = NSTextField(); fields[definition.key] = field; control = field
                if definition.kind == .tags || ["content", "text", "context", "source_query", "reflect_mission", "retain_mission", "retain_custom_instructions"].contains(definition.key) {
                    field.usesSingleLineMode = false; field.cell?.wraps = true
                    field.heightAnchor.constraint(equalToConstant: 72).isActive = true
                }
                if definition.key == "limit" { field.stringValue = "20" }
                if definition.key == "offset" { field.stringValue = "0" }
                if definition.key == "path", action == "list" { field.stringValue = "resources" }
            }
            let label = Build.label(L(definition.label), font: .systemFont(ofSize: 12), lines: 0)
            let row = Build.stack([label, control], spacing: 5)
            control.translatesAutoresizingMaskIntoConstraints = false; control.setAccessibilityLabel(L(definition.label))
            inputColumn.addArrangedSubview(row); row.widthAnchor.constraint(equalTo: inputColumn.widthAnchor).isActive = true
            control.widthAnchor.constraint(equalTo: row.widthAnchor).isActive = true
            label.widthAnchor.constraint(equalTo: row.widthAnchor).isActive = true
        }
    }
    private func fieldDefinitions() -> [Field] {
        guard let feature else { return [] }
        let id = Field("id", "Exact service-issued ID", required: true)
        let pagination = [Field("limit", "Result limit (1–20)", .integer), Field("offset", "Offset (0–100000)", .integer)]
        if backend == .pgvector || backend == .lanceDB { return [Field("document_id", "Exact document ID", required: true), Field("text", "Replacement document text", required: true)] }
        switch feature {
        case .bankProfile, .bankConfig:
            guard action == "update" else { return [] }
            return [Field("disposition_skepticism", "Skepticism (1–5)", .integer), Field("disposition_literalism", "Literalism (1–5)", .integer), Field("disposition_empathy", "Empathy (1–5)", .integer), Field("reflect_mission", "Reflect mission"), Field("retain_mission", "Retain mission"), Field("retain_chunk_size", "Retain chunk size", .integer), Field("retain_extraction_mode", "Extraction mode — exact service value"), Field("retain_custom_instructions", "Retain instructions"), Field("retain_extract_labels", "Extract labels (mixed leaves unchanged)", .boolean), Field("recall_max_tokens", "Recall maximum tokens", .integer), Field("recall_budget", "Recall budget", .budget), Field("reflect_max_iterations", "Reflect maximum iterations", .integer), Field("reflect_max_tokens", "Reflect maximum tokens", .integer)]
        case .directives, .mentalModels:
            if action == "list" { return pagination }
            if ["get", "delete", "refresh", "history"].contains(action) { return [id] }
            let mental = feature == .mentalModels
            var values: [Field] = action == "update" ? [id] : mental ? [Field("id", "Optional explicit mental-model ID")] : []
            values += [Field("name", "Name", required: action == "create"), Field(mental ? "source_query" : "content", mental ? "Source query" : "Directive content", required: action == "create"), Field("tags", "Tags — one per line", .tags)]
            values += mental ? [Field("max_tokens", "Maximum tokens", .integer)] : [Field("priority", "Priority", .integer), Field("is_active", "Active directive", .boolean)]
            return values
        case .mentalModelHistory: return [id]
        case .observations: return ["list", "scopes"].contains(action) ? pagination : action == "clear_derived" ? [id] : []
        case .memoryEdit:
            if action != "update" { return [id] }
            return [id, Field("text", "Replacement memory text"), Field("context", "Context"), Field("occurred_start", "Occurred start (ISO 8601)"), Field("occurred_end", "Occurred end (ISO 8601)"), Field("tags", "Tags — one per line", .tags)]
        case .memoryInvalidate, .memoryRestore: return action == "update" ? [id, Field("reason", "Reason")] : [id]
        case .documents: return action == "list" ? pagination : [id]
        case .sessions:
            if action == "list" { return [] }
            return action == "add_message" ? [id, Field("role", "Visible speaker", .role), Field("content", "Visible message text", required: true)] : [id]
        case .resources:
            switch action {
            case "list": return [Field("path", "Bot-relative resources path", required: true), Field("offset", "Offset (0–100000)", .integer)]
            case "add": return [id, Field("source_url", "Exact HTTP(S) source URL — no credentials/query/fragment", required: true)]
            default: return [Field("path", "Bot-relative resources path", required: true)]
            }
        case .tasks: return action == "list" ? (backend == .hindsight ? pagination : []) : [id]
        default: return []
        }
    }
    func command() throws -> MemoryAdvancedCommand {
        guard let feature, capabilities.supports(feature), actions(for: feature).contains(action) else { throw MemoryUIError.unsupported }
        var body: [String: Any] = [:]
        for definition in definitions {
            if let flag = flags[definition.key] {
                if flag.state != .mixed { body[definition.key] = flag.state == .on }
                continue
            }
            if let choice = choices[definition.key] {
                if definition.kind == .role || choice.indexOfSelectedItem > 0 { body[definition.key] = choice.titleOfSelectedItem }
                continue
            }
            let value = fields[definition.key]?.stringValue ?? ""
            if value.isEmpty { guard !definition.required else { throw MemoryUIError.invalidInput }; continue }
            guard value.utf8.count <= 32768 else { throw MemoryUIError.invalidInput }
            switch definition.kind {
            case .integer:
                guard let number = Int(value) else { throw MemoryUIError.invalidInput }
                if definition.key == "limit" { guard (1...20).contains(number) else { throw MemoryUIError.invalidInput } }
                if definition.key == "offset" { guard (0...100000).contains(number) else { throw MemoryUIError.invalidInput } }
                if definition.key.hasPrefix("disposition_") { guard (1...5).contains(number) else { throw MemoryUIError.invalidInput } }
                if definition.key == "max_tokens", feature == .mentalModels { guard (256...8192).contains(number) else { throw MemoryUIError.invalidInput } }
                body[definition.key] = number
            case .tags: body[definition.key] = value.components(separatedBy: .newlines).filter { !$0.isEmpty }
            default: body[definition.key] = value
            }
            if definition.key == "id" { try MemoryUIValidation.identifier(value) }
            if definition.key == "path" { try MemoryUIValidation.resourcePath(value, allowRoot: action == "list") }
            if definition.key == "source_url" { try MemoryUIValidation.endpoint(value, schemes: ["http", "https"]) }
        }
        if (feature == .bankProfile || feature == .bankConfig) && action == "update" {
            guard !body.isEmpty else { throw MemoryUIError.invalidInput }
            body = ["updates": body]
        }
        if feature == .mentalModels && ["create", "update"].contains(action) { body["trigger"] = ["refresh_after_consolidation": false, "refresh_cron": NSNull()] }
        if feature == .memoryEdit && action == "update" { guard body.count >= 2 else { throw MemoryUIError.invalidInput } }
        return .init(feature: feature, action: action, body: body)
    }
    override func confirmTapped() {
        guard !busy else { return }
        do {
            let command = try command()
            run { [self] in
                if !["list", "get", "read", "history", "chunks", "scopes"].contains(command.action) {
                    let details = try JSONSerialization.data(withJSONObject: command.body, options: [.prettyPrinted, .sortedKeys])
                    guard await confirm("Run this exact advanced action?", details: "Bot: \(botID)\nFeature: \(command.feature.rawValue)\nAction: \(command.action)\n\(String(decoding: details, as: UTF8.self))\n" + L("This can change service data or invoke paid downstream models. No background schedule is authorized. Deletes do not prove backup/history erasure.")) else { return }
                }
                output(try MemoryServiceResultFormatting.render(await service.advanced(botID: botID, command: command)))
            }
        } catch { showError(error) }
    }
}

enum MemoryServiceResultFormatting {
    static func render(_ data: Data) throws -> String {
        guard data.count <= 1_048_576, let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else { throw MemoryUIError.invalidReply }
        func redacted(_ value: Any) -> Any {
            if let object = value as? [String: Any] {
                return object.reduce(into: [String: Any]()) { result, entry in
                    let key = entry.key.lowercased()
                    if ["secret", "api_key", "password", "authorization", "preview_token", "token"].contains(key) { result[entry.key] = "<redacted>" }
                    else { result[entry.key] = redacted(entry.value) }
                }
            }
            if let array = value as? [Any] { return array.map(redacted) }
            return value
        }
        let safe = redacted(object)
        return String(decoding: try JSONSerialization.data(withJSONObject: safe, options: [.prettyPrinted, .sortedKeys]), as: UTF8.self)
    }
}
