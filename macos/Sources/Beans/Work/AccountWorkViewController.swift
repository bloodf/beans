import AppKit

/// Account work is fetched through the existing local, authenticated CLI connection.
@MainActor
final class AccountWorkViewController: NSViewController, NSTableViewDataSource, NSTableViewDelegate {
    private struct Revision: Decodable { let counter: UInt64; let device_id: String }
    private struct Source: Decodable { let chat_id: String }
    private struct Attention: Decodable {
        let id: String; let title: String; let summary: String; let next_action: String
        let resolved: Bool; let revision: Revision; let sources: [Source]
    }
    private struct AttentionView: Decodable { let items: [Attention] }
    private struct WorkTask: Decodable {
        let id: String; let revision: UInt64; let goal: String; let state: String
        let next_action: String; let chat_ids: [String]; let active_run: Run?
        struct Run: Decodable { let id: String }
    }
    private struct Handoff: Decodable {
        let handoff_id: String; let job_id: String; let delivery: String; let status: String?
        let request: Request; let report: Report?
        struct Request: Decodable {
            let from_bot_id: String; let source_chat_id: String; let target_chat_id: String
            let message: String; let expected_output: String; let acceptance_criteria: [String]
        }
        struct Report: Decodable { let summary: String; let evidence: [String] }
    }
    private struct Handoffs: Decodable { let handoffs: [Handoff] }
    private let store = AppStore.shared
    private let tabs = NSSegmentedControl(labels: [L("Attention"), L("Tasks"), L("Handoffs")], trackingMode: .selectOne, target: nil, action: nil)
    private let table = NSTableView()
    private let detail = NSTextView()
    private let status = NSTextField(wrappingLabelWithString: "")
    private let actionButton = NSButton(title: "", target: nil, action: nil)
    private let openButton = NSButton(title: L("Open Chat"), target: nil, action: nil)
    private let refreshButton = NSButton(title: L("Refresh"), target: nil, action: nil)
    private var attention: [Attention] = []
    private var tasks: [WorkTask] = []
    private var handoffs: [Handoff] = []
    private var busy = false
    private var generation = UUID()
    var onOpenChat: ((String) -> Void)?

    override func loadView() {
        view = NSView(frame: NSRect(x: 0, y: 0, width: 760, height: 600))
        tabs.selectedSegment = 0; tabs.target = self; tabs.action = #selector(tabChanged)
        refreshButton.target = self; refreshButton.action = #selector(refresh)
        actionButton.target = self; actionButton.action = #selector(performAction)
        openButton.target = self; openButton.action = #selector(openChat)
        let close = NSButton(title: L("Close"), target: self, action: #selector(closeSheet))
        close.keyEquivalent = "\u{1b}"
        let header = NSStackView(views: [tabs, refreshButton, close]); header.spacing = 12
        let column = NSTableColumn(identifier: .init("work")); column.title = L("Work")
        table.addTableColumn(column); table.headerView = nil; table.delegate = self; table.dataSource = self
        table.setAccessibilityLabel(L("Account work"))
        let list = NSScrollView(); list.documentView = table; list.hasVerticalScroller = true
        detail.isEditable = false; detail.isSelectable = true; detail.font = .systemFont(ofSize: 13)
        detail.autoresizingMask = [.width]; detail.isHorizontallyResizable = false
        let details = NSScrollView(); details.documentView = detail; details.hasVerticalScroller = true
        let actions = NSStackView(views: [openButton, actionButton]); actions.spacing = 12
        let stack = NSStackView(views: [header, status, list, details, actions])
        stack.orientation = .vertical; stack.alignment = .leading; stack.spacing = 12
        stack.translatesAutoresizingMaskIntoConstraints = false; view.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 20),
            stack.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -20),
            stack.topAnchor.constraint(equalTo: view.topAnchor, constant: 20),
            stack.bottomAnchor.constraint(equalTo: view.bottomAnchor, constant: -20),
            list.widthAnchor.constraint(equalTo: stack.widthAnchor), list.heightAnchor.constraint(equalToConstant: 200),
            details.widthAnchor.constraint(equalTo: stack.widthAnchor), details.heightAnchor.constraint(greaterThanOrEqualToConstant: 140),
            status.widthAnchor.constraint(equalTo: stack.widthAnchor)
        ])
        store.observe(self) { [weak self] event in
            if case .connectionChanged = event {
                self?.generation = UUID()
                self?.attention = []; self?.tasks = []; self?.handoffs = []
                self?.table.reloadData(); self?.updateSelection()
                self?.status.stringValue = L("Connection changed. Refresh to load current work.")
            }
        }
    }
    override func viewDidAppear() { super.viewDidAppear(); refresh() }
    @objc private func closeSheet() { generation = UUID(); dismiss(self) }
    @objc private func tabChanged() { refresh() }
    @objc private func refresh() {
        guard !busy else { return }
        attention = []; tasks = []; handoffs = []; table.reloadData(); detail.string = ""
        let tab = tabs.selectedSegment
        request {
            switch tab {
            case 0: self.attention = try await self.store.client.request("attention.list", as: AttentionView.self).items
            case 1: self.tasks = try await self.store.client.request("tasks.list", as: [WorkTask].self)
            default: self.handoffs = try await self.store.client.request("handoffs.list", as: Handoffs.self).handoffs
            }
        }
    }
    private func request(_ operation: @escaping () async throws -> Void) {
        guard store.isConnected, store.hasIdentity == true, !busy else { status.stringValue = L("Connect to the local CLI, then refresh."); return }
        busy = true; let token = generation; status.stringValue = L("Loading…"); updateSelection()
        Task { @MainActor in
            do {
                try await operation()
                guard self.generation == token else {
                    self.attention = []; self.tasks = []; self.handoffs = []
                    self.busy = false; self.table.reloadData(); self.updateSelection(); return
                }
                self.status.stringValue = self.numberOfRows(in: self.table) == 0 ? L("No work to show.") : L("Loaded. Refresh before acting on changed work.")
            } catch {
                guard self.generation == token else {
                    self.busy = false; self.updateSelection(); return
                }
                self.status.stringValue = L("Request failed: %@", error.localizedDescription)
            }
            self.busy = false; self.table.reloadData(); self.updateSelection()
        }
    }
    func numberOfRows(in tableView: NSTableView) -> Int {
        switch tabs.selectedSegment { case 0: attention.count; case 1: tasks.count; default: handoffs.count }
    }
    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let title: String
        switch tabs.selectedSegment {
        case 0: title = attention[row].title
        case 1: title = "[\(tasks[row].state)] \(tasks[row].goal)"
        default: title = "[\(handoffs[row].status ?? handoffs[row].delivery)] \(handoffs[row].request.message)"
        }
        return NSTextField(labelWithString: title)
    }
    func tableViewSelectionDidChange(_ notification: Notification) { updateSelection() }
    private var selectedRow: Int? {
        let row = table.selectedRow
        return row >= 0 && row < numberOfRows(in: table) ? row : nil
    }
    private func updateSelection() {
        refreshButton.isEnabled = !busy && store.isConnected; tabs.isEnabled = !busy
        openButton.isEnabled = false; actionButton.isEnabled = false; actionButton.title = L("Select Work")
        guard let row = selectedRow else { detail.string = L("Select work to inspect its details."); return }
        switch tabs.selectedSegment {
        case 0:
            let item = attention[row]
            detail.string = L("%@\n\nNext action: %@", item.summary, item.next_action)
            actionButton.title = L("Resolve"); actionButton.isEnabled = !item.resolved
        case 1:
            let item = tasks[row]
            detail.string = L("%@\n\nState: %@\nNext action: %@\nRevision: %@", item.goal, item.state, item.next_action, String(describing: item.revision))
            actionButton.title = L("Run Task")
            actionButton.isEnabled = item.active_run == nil && ["queued", "blocked"].contains(item.state)
        default:
            let item = handoffs[row]
            detail.string = "\(item.request.message)\n\n\(L("Expected output: %@", item.request.expected_output))\n\(item.request.acceptance_criteria.joined(separator: "\n"))\n\n\(item.report?.summary ?? L("No report received."))\n\(item.report?.evidence.joined(separator: "\n") ?? "")"
            actionButton.title = L("Cancel Handoff")
            actionButton.isEnabled = item.status == nil || item.status == "running" || item.status == "blocked"
        }
        actionButton.isEnabled = actionButton.isEnabled && !busy && store.isConnected
        openButton.isEnabled = !busy && store.isConnected && selectedChat != nil
    }
    private var selectedChat: String? {
        guard let row = selectedRow else { return nil }
        switch tabs.selectedSegment {
        case 0: return attention[row].sources.first?.chat_id
        case 1: return tasks[row].chat_ids.first
        default: return handoffs[row].request.target_chat_id
        }
    }
    @objc private func openChat() {
        guard let id = selectedChat, store.chat(id) != nil else { status.stringValue = L("The linked chat is unavailable on this Device."); return }
        dismiss(self); onOpenChat?(id)
    }
    @objc private func performAction() {
        guard let row = selectedRow, actionButton.isEnabled else { return }
        let confirmedGeneration = generation
        let alert = NSAlert(); alert.messageText = actionButton.title
        alert.informativeText = tabs.selectedSegment == 2 ? L("Cancel the current handoff attempt?") : L("Apply this action to the selected work?")
        alert.addButton(withTitle: L("Continue")); alert.addButton(withTitle: L("Cancel"))
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        guard generation == confirmedGeneration, store.isConnected, selectedRow == row else { return }
        let method: String; let params: [String: Any]
        switch tabs.selectedSegment {
        case 0:
            let item = attention[row]; method = "attention.resolve"
            params = ["id": item.id, "expected_revision": ["counter": item.revision.counter, "device_id": item.revision.device_id]]
        case 1:
            let item = tasks[row]; method = "tasks.run"
            params = ["id": item.id, "expected_revision": item.revision, "request_id": UUID().uuidString]
        default:
            let item = handoffs[row]; method = "handoffs.cancel"
            params = ["handoff_id": item.handoff_id, "job_id": item.job_id, "bot_id": item.request.from_bot_id, "reason": "Cancelled by the user in Beans."]
        }
        request {
            _ = try await self.store.client.request(method, params)
            // Reload authoritative state after a successful mutation; never predict its result.
            self.attention = []; self.tasks = []; self.handoffs = []
            switch self.tabs.selectedSegment {
            case 0: self.attention = try await self.store.client.request("attention.list", as: AttentionView.self).items
            case 1: self.tasks = try await self.store.client.request("tasks.list", as: [WorkTask].self)
            default: self.handoffs = try await self.store.client.request("handoffs.list", as: Handoffs.self).handoffs
            }
        }
    }
}
