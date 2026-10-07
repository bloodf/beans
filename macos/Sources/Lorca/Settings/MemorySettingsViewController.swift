import AppKit

final class MemorySettingsViewController: MemoryFormViewController {
    private let service: MemoryServiceAPI, setup: MemorySetupAPI
    private let bots: [(id: String, name: String)], runners: [(id: String, name: String)]
    private var inventory: MemoryServiceConnections?
    private let connectionMenu = NSPopUpButton(), embeddingMenu = NSPopUpButton()
    private let summary = Build.label(L("Loading masked account configuration…"), font: .systemFont(ofSize: 12), color: .secondaryLabelColor, lines: 0)
    private var editConnectionButton: MemoryActionButton!, disconnectButton: MemoryActionButton!
    private var editEmbeddingButton: MemoryActionButton!, removeEmbeddingButton: MemoryActionButton!, localAssetsButton: MemoryActionButton!
    convenience init() {
        let store = AppStore.shared
        self.init(service: store.memoryService, setup: store.memorySetup, bots: store.bots.map { ($0.id, $0.name) }, runners: store.runners.map { ($0.id, $0.name) })
    }
    init(service: MemoryServiceAPI, setup: MemorySetupAPI, bots: [(id: String, name: String)], runners: [(id: String, name: String)]) {
        self.service = service; self.setup = setup; self.bots = bots; self.runners = runners
        super.init(title: L("Memory connections"), subtitle: L("Account connections and portable embedding profiles. Local MEMORY.md stays independent."), width: 580)
    }
    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }
    override func loadView() {
        super.loadView()
        addForm(summary)
        heading("Memory connections")
        row("Connection", connectionMenu)
        connectionMenu.target = self; connectionMenu.action = #selector(selectionChanged)
        addForm(MemoryActionButton("Add connection") { [weak self] in self?.editConnection(new: true) })
        editConnectionButton = MemoryActionButton("Edit selected connection") { [weak self] in self?.editConnection(new: false) }
        disconnectButton = MemoryActionButton("Disconnect selected connection…") { [weak self] in self?.disconnect() }
        addForm(editConnectionButton); addForm(disconnectButton)
        heading("Embedding profiles")
        row("Profile", embeddingMenu); embeddingMenu.target = self; embeddingMenu.action = #selector(selectionChanged)
        addForm(MemoryActionButton("Add API / local CPU profile") { [weak self] in self?.editEmbedding(new: true) })
        editEmbeddingButton = MemoryActionButton("Replace selected profile metadata…") { [weak self] in self?.editEmbedding(new: false) }
        removeEmbeddingButton = MemoryActionButton("Remove selected profile…") { [weak self] in self?.removeEmbedding() }
        localAssetsButton = MemoryActionButton("Install local assets / check runtime…") { [weak self] in self?.localAssets() }
        addForm(editEmbeddingButton); addForm(removeEmbeddingButton); addForm(localAssetsButton)
        addForm(MemoryActionButton("Reload masked configuration") { [weak self] in self?.reload() })
        note("Remote services and API embeddings receive plaintext outside the encrypted relay. Their downstream model usage can cost money. Local LanceDB plus local CPU embeddings avoids remote transfer unless you explicitly enable a remote service operation. Capture and group capture are separate per-bot choices, off for new bots.")
        note("Names and secret-presence status are shown; endpoint credentials and private options are masked. Saving does not provision a service, initialize a schema, acquire assets or grant background spending. Disconnection does not delete remote banks. Backups and Lance history may retain deleted data.")
        setButtons(confirm: L("Done"), cancel: nil); selectionChanged()
    }
    override func viewDidAppear() { super.viewDidAppear(); if inventory == nil { reload() } }
    override func confirmTapped() { dismissSheet() }
    @objc private func selectionChanged() {
        let hasConnection = selectedConnection != nil, hasEmbedding = selectedEmbedding != nil
        editConnectionButton?.isEnabled = hasConnection; disconnectButton?.isEnabled = hasConnection
        editEmbeddingButton?.isEnabled = hasEmbedding; removeEmbeddingButton?.isEnabled = hasEmbedding
        localAssetsButton?.isEnabled = hasEmbedding && !runners.isEmpty
    }
    var selectedConnection: MemoryServiceConnectionView? {
        guard let id = connectionMenu.selectedItem?.representedObject as? String else { return nil }
        return inventory?.connections.first { $0.id == id }
    }
    var selectedEmbedding: MemoryServiceEmbeddingView? {
        guard let id = embeddingMenu.selectedItem?.representedObject as? String else { return nil }
        return inventory?.embeddings.first { $0.id == id }
    }
    func reload() {
        run { [self] in
            let previousConnection = selectedConnection?.id, previousEmbedding = selectedEmbedding?.id
            inventory = try await service.listConnections()
            connectionMenu.removeAllItems(); embeddingMenu.removeAllItems()
            for connection in inventory!.connections {
                connectionMenu.addItem(withTitle: "\(connection.name) · \(connection.backend.rawValue)")
                connectionMenu.lastItem?.representedObject = connection.id
                connectionMenu.lastItem?.toolTip = connection.id
                if connection.availability == .blocked {
                    connectionMenu.lastItem?.title += " · " + L("Unavailable: %@", connection.reason ?? MemoryUIError.unavailable.rawValue)
                }
            }
            for embedding in inventory!.embeddings {
                embeddingMenu.addItem(withTitle: "\(embedding.model) · \(embedding.dimensions) · \(embedding.id)")
                embeddingMenu.lastItem?.representedObject = embedding.id
            }
            if let item = connectionMenu.itemArray.first(where: { $0.representedObject as? String == previousConnection }) { connectionMenu.select(item) }
            if let item = embeddingMenu.itemArray.first(where: { $0.representedObject as? String == previousEmbedding }) { embeddingMenu.select(item) }
            summary.stringValue = inventory!.connections.isEmpty ? L("No connections yet. Add a connection, then bind and enable it from a bot's Memory Service page.") : L("%d masked connections · %d embedding profiles. Readiness is checked per bot, not inferred from this list.", inventory!.connections.count, inventory!.embeddings.count)
        }
    }
    private func child(_ controller: MemoryFormViewController) {
        controller.onSaved = { [weak self] in self?.reload() }
        presentAsSheet(controller)
    }
    private func editConnection(new: Bool) {
        guard !busy, new || selectedConnection != nil else { return }
        child(MemoryConnectionViewController(service: service, saved: new ? nil : selectedConnection, embeddings: inventory?.embeddings ?? [], bots: bots))
    }
    private func editEmbedding(new: Bool) {
        guard !busy, new || selectedEmbedding != nil else { return }
        child(MemoryEmbeddingViewController(service: service, saved: new ? nil : selectedEmbedding))
    }
    private func disconnect() {
        guard let connection = selectedConnection else { return }
        run { [self] in
            guard await confirm("Disconnect memory connection?", details: L("Disconnect %@ (%@) for this account and invalidate queued use. This does not erase its remote data.", connection.name, connection.id)) else { return }
            try await service.saved(service.send(.disconnectConnection(id: connection.id)))
            inventory = nil
        }
    }
    private func removeEmbedding() {
        guard let profile = selectedEmbedding else { return }
        run { [self] in
            guard await confirm("Remove embedding profile?", details: L("Remove %@ (%@). Connections using it require a replacement; existing vector data is not re-embedded or erased.", profile.model, profile.id)) else { return }
            try await service.saved(service.request("memory.embeddings.remove", ["id": profile.id]))
            inventory = nil
        }
    }
    private func localAssets() {
        guard !busy, let profile = selectedEmbedding else { return }
        child(MemoryLocalAssetsViewController(service: service, setup: setup, profile: profile, runners: runners))
    }
    override func didFinishWork() { selectionChanged(); if inventory == nil && errorLabel.stringValue.isEmpty { reload() } }
}
