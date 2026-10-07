import AppKit

final class MemoryEmbeddingViewController: MemoryFormViewController {
    private let service: MemoryServiceAPI
    private let saved: MemoryServiceEmbeddingView?
    private let profileID: String
    private var fields: [String: NSTextField] = [:]
    private var mode: NSPopUpButton!, normalization: NSPopUpButton!, distance: NSPopUpButton!, pooling: NSPopUpButton!
    private var specialTokens: NSButton!
    private let apiColumn = Build.stack([], spacing: 12), localColumn = Build.stack([], spacing: 12)
    private let secret: MemorySecretControl
    init(service: MemoryServiceAPI, saved: MemoryServiceEmbeddingView?) {
        self.service = service; self.saved = saved; profileID = saved?.id ?? "embedding-\(UUID().uuidString.lowercased())"
        secret = MemorySecretControl(hasSecret: saved?.hasSecret ?? false)
        super.init(title: L(saved == nil ? "New embedding profile" : "Replace embedding profile"), subtitle: L("Vector identity includes exact model revision and preprocessing. Changing it requires approved replacement-index re-embedding; dimensions alone are not compatibility."), width: 580)
    }
    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }
    override func loadView() {
        super.loadView()
        if saved != nil { note("The saved profile is masked. This form replaces all known metadata; fill every field from your model contract. Cancel preserves the existing profile.") }
        note("API embeddings send document/query plaintext to the specified endpoint and may incur provider costs. Local CPU uses only explicitly approved ONNX runtime, model and tokenizer assets; there is no automatic download or paid fallback.")
        mode = popup("Embedding mode", choices: ["Compatible API", "Local CPU (ONNX)"]); mode.target = self; mode.action = #selector(modeChanged)
        fields["model"] = text("Exact model", value: saved?.model ?? "")
        fields["revision"] = text("Pinned model revision", value: saved?.modelRevision ?? "")
        fields["dimensions"] = text("Exact dimensions", value: saved.map { String($0.dimensions) } ?? "")
        normalization = popup("Normalization", choices: ["none", "l2"])
        distance = popup("Distance", choices: ["cosine", "dot", "euclidean"])
        fields["document_prefix"] = text("Document prefix — exact text")
        fields["query_prefix"] = text("Query prefix — exact text")
        addForm(apiColumn)
        fields["endpoint"] = addText("Exact /embeddings endpoint", to: apiColumn)
        addRow("API key", view: secret, to: apiColumn)
        addForm(localColumn)
        for (key, label, value) in [
            ("model_sha256", "Model SHA-256", ""), ("tokenizer_sha256", "Tokenizer SHA-256", ""),
            ("max_tokens", "Maximum tokenizer tokens", ""), ("input_ids", "Input IDs tensor", ""),
            ("attention_mask", "Attention mask tensor", ""), ("token_type_ids", "Token type IDs tensor (optional)", ""),
            ("output", "Output tensor", ""), ("pad_id", "Padding token ID", ""),
            ("pad_type_id", "Padding type ID", ""), ("pad_token", "Padding token — exact text", ""),
        ] { fields[key] = addText(label, value: value, to: localColumn) }
        pooling = NSPopUpButton(); pooling.addItems(withTitles: ["mean", "cls", "pooled"]); addRow("Pooling", view: pooling, to: localColumn)
        specialTokens = NSButton(checkboxWithTitle: L("Add special tokens (per tokenizer contract)"), target: nil, action: nil)
        localColumn.addArrangedSubview(specialTokens)
        note("Save portable metadata first. Use Install Local Assets on the account page for exact Runner paths or approved HTTPS sources, license, sizes and hashes; setup requires a separate one-use preview approval.")
        setButtons(confirm: L("Save")); modeChanged()
    }
    private func addRow(_ title: String, view: NSView, to column: NSStackView) {
        let stack = Build.stack([Build.label(L(title), font: .systemFont(ofSize: 12)), view], spacing: 5)
        view.translatesAutoresizingMaskIntoConstraints = false; view.setAccessibilityLabel(L(title))
        column.addArrangedSubview(stack); stack.widthAnchor.constraint(equalTo: column.widthAnchor).isActive = true
        view.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
    }
    private func addText(_ title: String, value: String = "", to column: NSStackView) -> NSTextField {
        let field = NSTextField(string: value); addRow(title, view: field, to: column); return field
    }
    @objc private func modeChanged() { apiColumn.isHidden = mode.indexOfSelectedItem != 0; localColumn.isHidden = mode.indexOfSelectedItem == 0 }
    func profile() throws -> MemoryEmbeddingProfile {
        func value(_ key: String) -> String { fields[key]!.stringValue }
        guard let dimensions = UInt32(value("dimensions")) else { throw MemoryUIError.invalidInput }
        let api = mode.indexOfSelectedItem == 0
        var local: MemoryEmbeddingProfile.Local?
        if !api {
            guard let maxTokens = Int(value("max_tokens")), let padID = UInt32(value("pad_id")), let padTypeID = UInt32(value("pad_type_id")) else { throw MemoryUIError.invalidInput }
            local = .init(model_sha256: value("model_sha256"), tokenizer_sha256: value("tokenizer_sha256"), max_tokens: maxTokens, pooling: pooling.titleOfSelectedItem!, tensors: .init(input_ids: value("input_ids"), attention_mask: value("attention_mask"), token_type_ids: value("token_type_ids").isEmpty ? nil : value("token_type_ids"), output: value("output")), add_special_tokens: specialTokens.state == .on, pad_id: padID, pad_type_id: padTypeID, pad_token: value("pad_token"))
        }
        let profile = MemoryEmbeddingProfile(model: value("model"), revision: value("revision"), dimensions: dimensions, normalization: normalization.titleOfSelectedItem!, distance: distance.titleOfSelectedItem!, document_prefix: value("document_prefix"), query_prefix: value("query_prefix"), endpoint: api ? value("endpoint") : nil, mode: api ? .api : .localCPU, local: local)
        try profile.validate(); return profile
    }
    override func confirmTapped() {
        guard !busy else { return }
        do {
            let profile = try profile()
            let patch: MemoryServiceSecretPatch = profile.mode == .localCPU ? .clear : try secret.patch()
            run { [self] in
                if profile.endpoint?.hasPrefix("http:") == true {
                    guard await confirm("Save an insecure embedding endpoint?", details: L("Exact endpoint %@. Memory text and authentication cross plaintext HTTP when this profile is used. The connection also requires explicit insecure-HTTP approval.", profile.endpoint!)) else { return }
                }
                guard await confirm("Save this vector-space profile?", details: L("Profile %@, model %@, revision %@, dimensions %d. Exact endpoint: %@. Secret action: %@. Verify the selected secret belongs to this target. Existing indexes are not re-embedded or migrated by this action. Remote API processing can incur costs; local mode clears any API key. No fallback or download is enabled.", profileID, profile.model, profile.revision, Int(profile.dimensions), profile.endpoint ?? "local CPU", String(describing: patch))) else { return }
                try await service.saveEmbedding(id: profileID, profile: profile, secret: patch)
                finishSave()
            }
        } catch { showError(error) }
    }
}
