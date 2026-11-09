import AppKit
import CoreFoundation

/// Native forms retain structured CLI values without dropping unknown content keys.
@MainActor
final class WorkflowLibraryViewController: NSViewController, NSTableViewDataSource, NSTableViewDelegate {
    struct Operation {
        let title: String
        let method: String
        let fields: [(String, String)]
        let mutates: Bool
    }
    private let store = AppStore.shared
    private let sections = NSPopUpButton()
    private let operations = NSPopUpButton()
    private let form = NSStackView()
    private let table = NSTableView()
    private let detail = NSTextView()
    private let status = NSTextField(wrappingLabelWithString: "")
    private let submit = NSButton(title: L("Load"), target: nil, action: nil)
    private let cancel = NSButton(title: L("Cancel request"), target: nil, action: nil)
    private var inputs: [String: NSTextField] = [:]
    private var rows: [[String: Any]] = []
    private var requestTask: Task<Void, Never>?
    private var generation = UUID()
    private var sectionDrafts: [Int: [String: String]] = [:]
    private var previousSection = -1
    private var busy = false
    private var selected: [String: Any] = [:]
    private var draftFields: [String: String] = [:]
    private var current: Operation { Self.groups[sections.indexOfSelectedItem].1[operations.indexOfSelectedItem] }

    override func loadView() {
        view = NSView(frame: NSRect(x: 0, y: 0, width: 820, height: 720))
        sections.addItems(withTitles: Self.groups.map { L($0.0) })
        sections.target = self; sections.action = #selector(sectionChanged)
        operations.target = self; operations.action = #selector(operationChanged)
        submit.target = self; submit.action = #selector(send)
        cancel.target = self; cancel.action = #selector(cancelRequest)
        cancel.isEnabled = false
        let close = NSButton(title: L("Close"), target: self, action: #selector(closeSheet))
        close.keyEquivalent = "\u{1b}"
        let header = NSStackView(views: [sections, operations, close]); header.spacing = 10
        form.orientation = .vertical; form.alignment = .leading; form.spacing = 6
        let formScroll = NSScrollView(); formScroll.documentView = form; formScroll.hasVerticalScroller = true
        let column = NSTableColumn(identifier: .init("item")); column.title = L("Items")
        table.addTableColumn(column); table.delegate = self; table.dataSource = self
        table.setAccessibilityLabel(L("Workflow library items"))
        let list = NSScrollView(); list.documentView = table; list.hasVerticalScroller = true
        detail.isEditable = false; detail.isSelectable = true
        detail.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
        detail.autoresizingMask = [.width]
        let result = NSScrollView(); result.documentView = detail; result.hasVerticalScroller = true
        let buttons = NSStackView(views: [submit, cancel]); buttons.spacing = 10
        let stack = NSStackView(views: [header, formScroll, buttons, status, list, result])
        stack.orientation = .vertical; stack.alignment = .leading; stack.spacing = 10
        stack.translatesAutoresizingMaskIntoConstraints = false; view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 20),
            stack.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -20),
            stack.topAnchor.constraint(equalTo: view.topAnchor, constant: 20),
            stack.bottomAnchor.constraint(equalTo: view.bottomAnchor, constant: -20),
            formScroll.widthAnchor.constraint(equalTo: stack.widthAnchor),
            formScroll.heightAnchor.constraint(equalToConstant: 230),
            form.widthAnchor.constraint(equalTo: formScroll.contentView.widthAnchor),
            list.widthAnchor.constraint(equalTo: stack.widthAnchor), list.heightAnchor.constraint(equalToConstant: 110),
            result.widthAnchor.constraint(equalTo: stack.widthAnchor), result.heightAnchor.constraint(greaterThanOrEqualToConstant: 140),
            status.widthAnchor.constraint(equalTo: stack.widthAnchor)
        ])
        store.observe(self) { [weak self] event in
            switch event {
            case .identityChanged, .connectionChanged: self?.invalidateAccount()
            default: break
            }
        }
        sectionChanged()
    }
    override func viewWillDisappear() { super.viewWillDisappear(); cancelRequest() }
    private func invalidateAccount() {
        cancelRequest(); rows = []; selected = [:]; draftFields = [:]
        sectionDrafts = [:]; inputs.values.forEach { $0.stringValue = "" }
        table.reloadData(); detail.string = ""; operationChanged()
        status.stringValue = L("Account or connection changed. Load current data again.")
    }
    @objc private func cancelRequest() {
        generation = UUID(); requestTask?.cancel(); requestTask = nil; setBusy(false)
        status.stringValue = L("Request canceled locally. A submitted mutation may have completed. Load before retrying.")
    }
    @objc private func closeSheet() {
        if busy || inputs.values.contains(where: { !$0.stringValue.isEmpty }) || draftFields.values.contains(where: { !$0.isEmpty }) || sectionDrafts.values.contains(where: { $0.values.contains(where: { !$0.isEmpty }) }) {
            let alert = NSAlert(); alert.messageText = L("Close this form?")
            alert.informativeText = L("Unsaved fields will be discarded. Submitted actions may already have completed.")
            alert.addButton(withTitle: L("Keep editing")); alert.addButton(withTitle: L("Close"))
            guard alert.runModal() == .alertSecondButtonReturn else { return }
        }
        cancelRequest(); dismiss(self)
    }
    @objc private func sectionChanged() {
        cancelRequest()
        if previousSection >= 0 {
            for (key, field) in inputs { draftFields[key] = field.stringValue }
            sectionDrafts[previousSection] = draftFields
        }
        previousSection = sections.indexOfSelectedItem
        inputs = [:]; selected = [:]; draftFields = sectionDrafts[previousSection] ?? [:]
        rows = []; table.reloadData(); detail.string = ""
        operations.removeAllItems(); operations.addItems(withTitles: Self.groups[sections.indexOfSelectedItem].1.map { L($0.title) })
        operationChanged()
    }
    @objc private func operationChanged() {
        for (key, field) in inputs { draftFields[key] = field.stringValue }
        inputs = [:]
        for child in form.arrangedSubviews { form.removeArrangedSubview(child); child.removeFromSuperview() }
        for (key, hint) in current.fields {
            let label = NSTextField(labelWithString: key + " · " + L(hint))
            let field = NSTextField(); field.placeholderString = L(hint); field.setAccessibilityLabel(key + " · " + L(hint))
            field.stringValue = draftFields[key] ?? ""
            let row = NSStackView(views: [label, field]); row.orientation = .vertical; row.alignment = .leading
            form.addArrangedSubview(row)
            row.widthAnchor.constraint(equalTo: form.widthAnchor).isActive = true
            field.widthAnchor.constraint(equalTo: row.widthAnchor).isActive = true
            inputs[key] = field
        }
        submit.title = L(current.title); status.stringValue = ""
    }
    private func setBusy(_ value: Bool) {
        busy = value; submit.isEnabled = !value; cancel.isEnabled = value
        sections.isEnabled = !value; operations.isEnabled = !value; table.isEnabled = !value
        inputs.values.forEach { $0.isEnabled = !value }
    }
    private func parameters() throws -> [String: Any] {
        var result: [String: Any] = [:]
        for (key, hint) in current.fields {
            let text = inputs[key]?.stringValue ?? ""
            if text.isEmpty {
                if key == "expected_hash", current.method == "playbooks.save" { result[key] = ""; continue }
                if hint.hasPrefix("optional") { continue }
                throw NSError(domain: "WorkflowLibrary", code: 1, userInfo: [NSLocalizedDescriptionKey: L("Enter %@", key)])
            }
            if hint.contains("JSON") {
                result[key] = try JSONSerialization.jsonObject(with: Data(text.utf8), options: [.fragmentsAllowed])
            } else { result[key] = text }
        }
        if current.method.hasPrefix("templates.import") {
            let count = ["path", "link"].filter { result[$0] != nil }.count
            guard count == 1 else { throw formError(L("Choose exactly one template path or shared link.")) }
        }
        if let scope = result["scope"] {
            guard let object = scope as? [String: Any],
                  let kind = object["kind"] as? String, ["bot", "project"].contains(kind),
                  let id = object["id"] as? String, !id.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                throw formError(L("Scope needs kind bot or project and a nonempty id."))
            }
        }
        for key in ["answers", "bot_ids", "selection", "content", "mappings", "file"] where result[key] != nil {
            guard result[key] is [String: Any] else { throw formError(L("%@ must be a JSON object.", key)) }
        }
        if let reviewed = result["reviewed"] {
            guard let number = reviewed as? NSNumber, CFGetTypeID(number) == CFBooleanGetTypeID(), number.boolValue else {
                throw formError(L("Review the preview, then enter true to confirm reviewed content."))
            }
        }
        return result
    }
    private func formError(_ text: String) -> NSError {
        NSError(domain: "WorkflowLibrary", code: 2, userInfo: [NSLocalizedDescriptionKey: text])
    }
    @objc private func send() {
        guard !busy, let owner = store.deviceNameOwner, let identity = store.identityID else {
            status.stringValue = L("Wait for the current account connection."); return
        }
        do {
            let params = try parameters(); let operation = current
            if operation.mutates {
                let alert = NSAlert(); alert.messageText = L("Confirm %@?", L(operation.title))
                alert.informativeText = L("Review the displayed result and form before continuing. The CLI validates permissions and current state.")
                alert.addButton(withTitle: L("Cancel")); alert.addButton(withTitle: L(operation.title))
                guard alert.runModal() == .alertSecondButtonReturn else { return }
            }
            guard owner == store.deviceNameOwner, identity == store.identityID else { invalidateAccount(); return }
            generation = UUID(); let token = generation; setBusy(true)
            requestTask = Task { [weak self] in
                guard let self else { return }
                defer { if token == generation { setBusy(false); requestTask = nil } }
                do {
                    try Task.checkCancellation()
                    guard token == generation, owner == store.deviceNameOwner, identity == store.identityID else { return }
                    let data = try await store.client.request(operation.method, params)
                    try Task.checkCancellation()
                    guard token == generation, owner == store.deviceNameOwner, identity == store.identityID else { return }
                    let value = try JSONSerialization.jsonObject(with: data, options: [.fragmentsAllowed])
                    detail.string = Self.pretty(value)
                    rows = Self.items(value)
                    if Self.groups[sections.indexOfSelectedItem].0 == "Assets" {
                        rows = rows.filter { $0["kind"] as? String == "asset" }
                    }
                    if let object = value as? [String: Any], let setup = object["setup"] as? [String: Any] {
                        rows = [setup]
                    }
                    table.reloadData()
                    status.stringValue = L("Completed %@. Load again to see authoritative state.", L(operation.title))
                } catch {
                    guard token == generation, owner == store.deviceNameOwner, identity == store.identityID else { return }
                    status.stringValue = error.localizedDescription
                }
            }
        } catch { status.stringValue = error.localizedDescription }
    }
    static func pretty(_ value: Any) -> String {
        guard let data = try? JSONSerialization.data(withJSONObject: value, options: [.prettyPrinted, .sortedKeys, .fragmentsAllowed]),
              let text = String(data: data, encoding: .utf8) else { return L("Unreadable response") }
        return text
    }
    static func items(_ value: Any) -> [[String: Any]] {
        if let array = value as? [[String: Any]] { return array }
        guard let object = value as? [String: Any] else { return [] }
        for key in ["entries", "playbooks", "workflow_setups", "packs", "workflows", "skills", "bots"] {
            if let array = object[key] as? [[String: Any]] { return array }
        }
        return [object]
    }
    func numberOfRows(in tableView: NSTableView) -> Int { rows.count }
    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let item = rows[row]
        return NSTextField(labelWithString: item["name"] as? String ?? item["title"] as? String ?? item["id"] as? String ?? L("Result"))
    }
    func tableViewSelectionDidChange(_ notification: Notification) {
        guard rows.indices.contains(table.selectedRow), !busy else { return }
        selected = rows[table.selectedRow]; detail.string = Self.pretty(selected)
        for key in ["id", "entry_id", "pack_id", "content", "scope", "provenance"] {
            let value = selected[key] ?? ((key == "entry_id" || key == "pack_id") ? selected["id"] : nil)
            if let value { draftFields[key] = value as? String ?? Self.pretty(value) }
        }
        if let revision = selected["revision"] { draftFields["expected_revision"] = Self.pretty(revision) }
        if let hash = selected["hash"] as? String { draftFields["expected_hash"] = hash }
        // Apply selected authoritative values without replacing fields the user is editing.
        for (key, field) in inputs where field.stringValue.isEmpty { field.stringValue = draftFields[key] ?? "" }
    }
}

extension WorkflowLibraryViewController {
    static let groups: [(String, [Operation])] = [
        ("Workflows", [
            Operation(title: "List saved workflows", method: "workflows.list", fields: [("bot_id", "optional bot ID")], mutates: false),
            Operation(title: "Create saved workflow", method: "workflows.create", fields: [("bot_id", "bot ID"), ("name", "Name"), ("definition", "JSON definition")], mutates: true),
            Operation(title: "Delete saved workflow", method: "workflows.delete", fields: [("id", "Workflow ID")], mutates: true),
            Operation(title: "Browse packs", method: "marketplace", fields: [], mutates: false),
            Operation(title: "Start setup", method: "workflows.start", fields: [("runner_id", "Runner ID"), ("pack_id", "Pack ID")], mutates: true),
            Operation(title: "Load setup", method: "workflows.get", fields: [("id", "Setup ID")], mutates: false),
            Operation(title: "Configure setup", method: "workflows.configure", fields: [("id", "Setup ID"), ("answers", "JSON object of question IDs to answers"), ("bot_ids", "optional JSON specialist IDs to bot IDs")], mutates: true),
            Operation(title: "Connect service", method: "workflows.connection", fields: [("id", "Setup ID"), ("service_id", "Service ID"), ("plugin_id", "optional installed connection ID"), ("account_name", "optional account name")], mutates: true),
            Operation(title: "Run sample", method: "workflows.sample", fields: [("id", "Setup ID")], mutates: true),
            Operation(title: "Review sample", method: "workflows.review", fields: [("id", "Setup ID"), ("job_id", "Current completed sample job ID")], mutates: true),
            Operation(title: "Enable schedules", method: "workflows.enable", fields: [("id", "Setup ID")], mutates: true),
            Operation(title: "Cancel setup", method: "workflows.cancel", fields: [("id", "Setup ID")], mutates: true),
        ]),
        ("Playbooks", [
            Operation(title: "List playbooks", method: "bootstrap", fields: [], mutates: false),
            Operation(title: "Load playbook", method: "playbooks.get", fields: [("scope", "JSON object with kind bot or project and id"), ("id", "Playbook ID")], mutates: false),
            Operation(title: "Save playbook", method: "playbooks.save", fields: [("scope", "JSON object with kind bot or project and id"), ("id", "optional existing playbook ID"), ("content", "JSON name, description, instructions, examples, references, scripts"), ("expected_revision", "JSON integer, 0 for new"), ("expected_hash", "optional current hash, empty for new"), ("provenance", "optional JSON provenance")], mutates: true),
            Operation(title: "Remove playbook", method: "playbooks.remove", fields: [("scope", "JSON scope"), ("id", "Playbook ID"), ("expected_revision", "JSON integer"), ("expected_hash", "Current hash")], mutates: true),
            Operation(title: "Export playbook", method: "playbooks.export", fields: [("scope", "JSON scope"), ("id", "Playbook ID")], mutates: false),
        ]),
        ("Templates", [
            Operation(title: "List bot contents", method: "templates.contents", fields: [("bot_id", "Bot ID")], mutates: false),
            Operation(title: "Preview export", method: "templates.export.preview", fields: [("bot_id", "Bot ID"), ("selection", "JSON profile, skill_ids, memory_ids, routine_ids, requirement_ids")], mutates: false),
            Operation(title: "Export reviewed template", method: "templates.export", fields: [("bot_id", "Bot ID"), ("selection", "JSON selection"), ("path", "Local output path"), ("expected_digest", "Preview digest"), ("reviewed", "JSON true"), ("overwrite", "optional JSON boolean")], mutates: true),
            Operation(title: "Preview import", method: "templates.import.preview", fields: [("path", "optional local template path"), ("link", "optional shared link"), ("name", "New bot name"), ("runner_id", "Runner ID"), ("mappings", "optional JSON service mappings")], mutates: false),
            Operation(title: "Import reviewed template", method: "templates.import", fields: [("path", "optional local template path"), ("link", "optional shared link"), ("name", "New bot name"), ("runner_id", "Runner ID"), ("mappings", "optional JSON service mappings"), ("expected_digest", "Preview digest"), ("reviewed", "JSON true")], mutates: true),
            Operation(title: "Share reviewed template", method: "templates.share", fields: [("bot_id", "Bot ID"), ("selection", "JSON selection"), ("expected_digest", "Preview digest"), ("reviewed", "JSON true"), ("link_id", "optional existing link ID")], mutates: true),
            Operation(title: "Revoke shared link", method: "templates.unshare", fields: [("link_id", "Link ID")], mutates: true),
        ]),
        ("Projects", [
            Operation(title: "List project entries", method: "projects.get", fields: [("chat_id", "Group chat ID"), ("entry_id", "optional entry ID"), ("history", "optional JSON boolean"), ("after", "optional page cursor"), ("limit", "optional JSON integer")], mutates: false),
            Operation(title: "Save project entry", method: "projects.save", fields: [("chat_id", "Group chat ID"), ("kind", "brief, goal, constraint, decision, fact, document, asset"), ("title", "Title"), ("text", "optional content"), ("source", "optional JSON source"), ("verification", "optional verification"), ("supersedes", "optional JSON array of revision IDs"), ("removed", "optional JSON boolean")], mutates: true),
            Operation(title: "Refresh project source", method: "projects.refresh", fields: [("chat_id", "Group chat ID"), ("entry_id", "Entry ID")], mutates: true),
        ]),
        ("Assets", [
            Operation(title: "List project assets", method: "projects.get", fields: [("chat_id", "Group chat ID"), ("entry_id", "optional entry ID")], mutates: false),
            Operation(title: "Add reference asset", method: "projects.asset", fields: [("chat_id", "Group chat ID"), ("file", "JSON existing Attachment reference"), ("title", "optional title"), ("text", "optional content")], mutates: true),
            Operation(title: "Resolve asset path", method: "projects.asset_path", fields: [("chat_id", "Group chat ID"), ("entry_id", "Asset entry ID")], mutates: false),
        ]),
    ]
}
