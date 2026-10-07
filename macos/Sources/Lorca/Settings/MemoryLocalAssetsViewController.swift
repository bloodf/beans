import AppKit

final class MemoryLocalAssetsViewController: MemoryFormViewController {
    private let service: MemoryServiceAPI, setup: MemorySetupAPI
    private let profile: MemoryServiceEmbeddingView
    private let runnerMenu = NSPopUpButton()
    private var assets: [MemoryAssetForm] = []
    private var approval: MemoryLocalAssetApproval?
    private let approved = NSButton(checkboxWithTitle: L("I approve these exact assets, licenses and runtime load"), target: nil, action: nil)
    private let previewText = Build.label("", font: .monospacedSystemFont(ofSize: 11, weight: .regular), lines: 0)
    private var previewButton: MemoryActionButton!
    init(service: MemoryServiceAPI, setup: MemorySetupAPI, profile: MemoryServiceEmbeddingView, runners: [(id: String, name: String)]) {
        self.service = service; self.setup = setup; self.profile = profile
        super.init(title: L("Install local embedding assets"), subtitle: L("Only the explicitly previewed three files are acquired. Installing an asset and proving a ready runtime are distinct."), width: 640)
        for runner in runners { runnerMenu.addItem(withTitle: "\(runner.name) · \(runner.id)"); runnerMenu.lastItem?.representedObject = runner.id }
    }
    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }
    override func loadView() {
        super.loadView()
        note("Profile: \(profile.id). Model: \(profile.model), revision: \(profile.modelRevision). Save a local CPU profile with matching model/tokenizer hashes first. Paths refer to the selected Runner, not necessarily this Mac.")
        row("Selected Runner", runnerMenu)
        for kind in [MemoryAssetKind.runtime, .model, .tokenizer] {
            heading(kind.rawValue.capitalized)
            let form = MemoryAssetForm(kind: kind); assets.append(form); addForm(form)
        }
        note("Supplied files stay Runner-local. Download sources must be exact HTTPS URLs approved by that Runner's administrator origin allowlist; no redirects, retries, extraction, automatic downloads or paid fallback. Runtime ABI/signing and the model tensor contract must match the Runner.")
        previewButton = MemoryActionButton("Preview exact assets") { [weak self] in self?.preview() }
        addForm(previewButton); addForm(previewText); addForm(approved)
        approved.target = self; approved.action = #selector(approvalChanged)
        addForm(MemoryActionButton("Check approved runtime status") { [weak self] in self?.status() })
        setButtons(confirm: L("Install approved assets")); confirmButton.isEnabled = false
    }
    @objc private func approvalChanged() { confirmButton.isEnabled = approval != nil && approved.state == .on }
    private func currentTarget() async throws -> MemoryLocalAssetTarget {
        guard let runner = runnerMenu.selectedItem?.representedObject as? String else { throw MemoryUIError.invalidInput }
        let current = try await service.listConnections()
        guard let selected = current.embeddings.first(where: { $0.id == profile.id }) else { throw MemoryUIError.staleTarget }
        return .init(runnerID: runner, profileID: profile.id, profileRevision: selected.revision)
    }
    func preview() {
        guard !busy else { return }
        if approval != nil {
            approval = nil; approved.state = .off; previewText.stringValue = ""
            didFinishWork()
            return
        }
        approval = nil; approved.state = .off; confirmButton.isEnabled = false
        do {
            let plan = MemoryAssetPlan(assets: try assets.map { try $0.asset() })
            try plan.validate()
            run { [self] in
                let target = try await currentTarget()
                let preview = try await setup.previewLocal(target: target, plan: plan)
                approval = MemoryLocalAssetApproval(preview: preview)
                previewText.stringValue = Self.describe(preview)
            }
        } catch { showError(error) }
    }
    static func describe(_ preview: MemoryLocalAssetPreview) -> String {
        var lines = ["Runner: \(preview.runnerID)", "Profile: \(preview.profileID)", "Profile revision: \(preview.profileRevision.counter) · \(preview.profileRevision.deviceID)", "Total bytes: \(preview.totalBytes)", "Approval lifetime: \(preview.expiresInSeconds) seconds"]
        for asset in preview.assets {
            let source: String
            switch asset.source { case let .supplied(path): source = "Supplied: \(path)"; case let .download(url): source = "Download: \(url)" }
            lines += ["", asset.kind.rawValue, source, "License: \(asset.license)", "Exact bytes: \(asset.bytes)", "SHA-256: \(asset.sha256)"]
        }
        return lines.joined(separator: "\n")
    }
    override func confirmTapped() {
        guard !busy, approved.state == .on, let approval else { showError(MemoryUIError.confirmationRequired); return }
        self.approval = nil; approved.state = .off; confirmButton.isEnabled = false
        run { [self] in
            guard await confirm("Acquire and load these exact assets?", details: Self.describe(approval.preview)) else { return }
            let result = try await setup.applyLocal(approval, confirm: true, current: currentTarget())
            output(result.status == .ready ? L("Approved files installed; the explicit runtime/model load is ready. No inference was run.") : L("Approved files installed, but the native runtime is unavailable. This is not ready, and no fallback was attempted."))
            onSaved?()
        }
    }
    private func status() {
        run { [self] in
            let target = try await currentTarget()
            let status = try await setup.localStatus(runnerID: target.runnerID, profileID: target.profileID)
            output(L("Local runtime status: %@. This explicit check may load only a previously approved private binding; it does not acquire assets or run inference.", status.status.rawValue))
        }
    }
    override func didFinishWork() {
        runnerMenu.isEnabled = approval == nil
        for asset in assets { controls(in: asset).forEach { $0.isEnabled = approval == nil } }
        previewButton.title = L(approval == nil ? "Preview exact assets" : "Discard preview / edit assets")
        confirmButton.isEnabled = approval != nil && approved.state == .on
    }
}

private final class MemoryAssetForm: NSStackView {
    let kind: MemoryAssetKind
    let source = NSPopUpButton(), location = NSTextField(), license = NSTextField(), bytes = NSTextField(), hashField = NSTextField()
    init(kind: MemoryAssetKind) {
        self.kind = kind; super.init(frame: .zero)
        orientation = .vertical; alignment = .leading; spacing = 7; translatesAutoresizingMaskIntoConstraints = false
        source.addItems(withTitles: [L("Supplied absolute path on Runner"), L("Explicit HTTPS download")])
        for (label, view) in [("Source", source as NSView), ("Exact path / URL", location), ("License", license), ("Exact byte count", bytes), ("Full lowercase SHA-256", hashField)] {
            let stack = Build.stack([Build.label(L(label), font: .systemFont(ofSize: 12)), view], spacing: 4)
            view.translatesAutoresizingMaskIntoConstraints = false; view.setAccessibilityLabel("\(kind.rawValue) \(L(label))")
            addArrangedSubview(stack); stack.widthAnchor.constraint(equalTo: widthAnchor).isActive = true
            view.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
        }
    }
    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }
    func asset() throws -> MemoryAssetRequest {
        guard let bytes = UInt64(bytes.stringValue), bytes > 0 else { throw MemorySetupError.invalidAssetPlan }
        let assetSource: MemoryAssetSource
        if source.indexOfSelectedItem == 0 {
            guard location.stringValue.hasPrefix("/") else { throw MemorySetupError.invalidAssetSource }
            assetSource = .supplied(path: location.stringValue)
        } else { assetSource = .download(url: location.stringValue) }
        let asset = MemoryAssetRequest(kind: kind, source: assetSource, license: license.stringValue, bytes: bytes, sha256: hashField.stringValue)
        try asset.validate(); return asset
    }
}
