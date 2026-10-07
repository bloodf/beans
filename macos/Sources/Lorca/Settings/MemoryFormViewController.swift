import AppKit

final class MemoryActionButton: NSButton {
    var handler: (() -> Void)?
    init(_ title: String, handler: (() -> Void)? = nil) {
        self.handler = handler
        super.init(frame: .zero)
        self.title = L(title); bezelStyle = .rounded; target = self; action = #selector(invoke)
        translatesAutoresizingMaskIntoConstraints = false
    }
    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }
    @objc private func invoke() { handler?() }
}

final class MemorySecretControl: NSStackView {
    let mode = NSPopUpButton()
    let field = NSSecureTextField()
    init(hasSecret: Bool) {
        super.init(frame: .zero)
        orientation = .vertical; alignment = .leading; spacing = 6
        translatesAutoresizingMaskIntoConstraints = false
        mode.addItems(withTitles: [L(hasSecret ? "Keep stored secret (masked)" : "Keep — no stored secret"), L("Replace secret"), L("Clear secret")])
        mode.target = self; mode.action = #selector(changed)
        field.placeholderString = L("Enter replacement only")
        field.setAccessibilityLabel(L("Replacement secret"))
        field.translatesAutoresizingMaskIntoConstraints = false
        mode.translatesAutoresizingMaskIntoConstraints = false
        addArrangedSubview(mode); addArrangedSubview(field)
        mode.widthAnchor.constraint(equalTo: widthAnchor).isActive = true
        field.widthAnchor.constraint(equalTo: widthAnchor).isActive = true
        field.isEnabled = false
    }
    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }
    @objc private func changed() { field.isEnabled = mode.indexOfSelectedItem == 1 }
    func patch() throws -> MemoryServiceSecretPatch {
        switch mode.indexOfSelectedItem {
        case 0: return .keep
        case 2: return .clear
        default:
            guard !field.stringValue.isEmpty, field.stringValue.utf8.count <= 8192 else { throw MemoryServiceDraftError.invalidSecret }
            return .replace(field.stringValue)
        }
    }
}

/// Fixed native sheet chrome around a vertically scrolling form. Busy state fences every
/// control and dismissal path; failed writes leave the form and secure entry intact.
class MemoryFormViewController: SheetViewController {
    let formScroll = NSScrollView()
    let formColumn = Build.stack([], spacing: 14)
    let errorLabel = Build.label("", font: Theme.Font.caption, color: .systemRed, lines: 0)
    private(set) var busy = false
    private(set) var pendingTask: Task<Void, Never>?
    var onSaved: (() -> Void)?
    var confirmationHandler: ((String, String) async -> Bool)?
    var resultHandler: ((String) -> Void)?
    private var enabledBeforeBusy: [(NSControl, Bool)] = []
    private var editableBeforeBusy: [(NSTextView, Bool)] = []
    override var canDismiss: Bool { !busy && presentedViewControllers?.isEmpty != false }

    override func loadView() {
        super.loadView()
        formScroll.drawsBackground = false; formScroll.hasVerticalScroller = true
        formScroll.autohidesScrollers = true; formScroll.translatesAutoresizingMaskIntoConstraints = false
        let document = FlippedView()
        document.translatesAutoresizingMaskIntoConstraints = false
        formColumn.edgeInsets = NSEdgeInsets(top: 6, left: 4, bottom: 12, right: 4)
        document.addSubview(formColumn); formScroll.documentView = document
        NSLayoutConstraint.activate([
            document.widthAnchor.constraint(equalTo: formScroll.contentView.widthAnchor),
            formColumn.topAnchor.constraint(equalTo: document.topAnchor),
            formColumn.bottomAnchor.constraint(equalTo: document.bottomAnchor),
            formColumn.leadingAnchor.constraint(equalTo: document.leadingAnchor),
            formColumn.trailingAnchor.constraint(equalTo: document.trailingAnchor),
        ])
        contentStack.addArrangedSubview(formScroll); contentStack.addArrangedSubview(errorLabel)
        NSLayoutConstraint.activate([
            formScroll.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            formScroll.heightAnchor.constraint(equalToConstant: 440),
            errorLabel.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
        ])
    }
    override func viewDidAppear() {
        super.viewDidAppear()
        if let first = controls(in: formColumn).first(where: { $0.isEnabled && $0.acceptsFirstResponder }) {
            view.window?.makeFirstResponder(first)
        }
    }
    func addForm(_ view: NSView) {
        view.translatesAutoresizingMaskIntoConstraints = false
        formColumn.addArrangedSubview(view)
        view.widthAnchor.constraint(equalTo: formColumn.widthAnchor, constant: -8).isActive = true
    }
    func heading(_ text: String) { addForm(Build.label(L(text), font: .systemFont(ofSize: 13, weight: .semibold))) }
    func note(_ text: String) { addForm(Build.label(L(text), font: .systemFont(ofSize: 12), color: .secondaryLabelColor, lines: 0)) }
    func row(_ title: String, _ control: NSView) {
        let label = Build.label(L(title), font: .systemFont(ofSize: 12, weight: .medium), lines: 0)
        control.translatesAutoresizingMaskIntoConstraints = false
        control.setAccessibilityLabel(L(title))
        let stack = Build.stack([label, control], spacing: 5)
        control.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
        label.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
        addForm(stack)
    }
    @discardableResult func text(_ title: String, value: String = "", placeholder: String? = nil) -> NSTextField {
        let field = NSTextField(string: value)
        field.placeholderString = placeholder.map { L($0) }; row(title, field)
        return field
    }
    @discardableResult func popup(_ title: String, choices: [String]) -> NSPopUpButton {
        let popup = NSPopUpButton(); popup.addItems(withTitles: choices.map { L($0) }); row(title, popup); return popup
    }
    @discardableResult func toggle(_ title: String, on: Bool = false) -> NSButton {
        let button = NSButton(checkboxWithTitle: L(title), target: nil, action: nil)
        button.state = on ? .on : .off; addForm(button); return button
    }
    func output(_ text: String) {
        if let resultHandler { resultHandler(text); return }
        let controller = MemoryResultViewController(text: text)
        presentAsSheet(controller)
    }
    func showError(_ error: Error) {
        // RPC errors may contain untrusted backend payloads. Show known local errors only.
        if let local = error as? MemoryUIError { errorLabel.stringValue = L(local.errorDescription ?? "Memory request failed") }
        else if let draft = error as? MemoryServiceDraftError { errorLabel.stringValue = L(draft.rawValue.replacingOccurrences(of: "_", with: " ")) }
        else if let setup = error as? MemorySetupError { errorLabel.stringValue = L(setup.rawValue.replacingOccurrences(of: "_", with: " ")) }
        else { errorLabel.stringValue = L("Memory request failed. Check the connection or obtain a fresh preview, then try again. Your draft is preserved.") }
        NSAccessibility.post(element: errorLabel, notification: .valueChanged)
    }
    func run(_ work: @escaping @MainActor () async throws -> Void) {
        guard !busy else { return }
        busy = true; errorLabel.stringValue = L("Working…")
        enabledBeforeBusy = controls(in: view).map { ($0, $0.isEnabled) }
        enabledBeforeBusy.forEach { $0.0.isEnabled = false }
        editableBeforeBusy = textEditors(in: view).map { ($0, $0.isEditable) }
        editableBeforeBusy.forEach { $0.0.isEditable = false }
        pendingTask = Task { [weak self] in
            guard let self else { return }
            do { try await work(); self.errorLabel.stringValue = "" }
            catch { self.showError(error) }
            self.busy = false
            self.enabledBeforeBusy.forEach { $0.0.isEnabled = $0.1 }
            self.enabledBeforeBusy.removeAll()
            self.editableBeforeBusy.forEach { $0.0.isEditable = $0.1 }
            self.editableBeforeBusy.removeAll()
            self.didFinishWork()
        }
    }
    func didFinishWork() {}
    func confirm(_ title: String, details: String) async -> Bool {
        if let confirmationHandler { return await confirmationHandler(title, details) }
        guard let window = view.window else { return false }
        let alert = NSAlert(); alert.messageText = L(title); alert.informativeText = details
        alert.addButton(withTitle: L("Confirm")); alert.addButton(withTitle: L("Cancel"))
        return await withCheckedContinuation { continuation in
            alert.beginSheetModal(for: window) { response in continuation.resume(returning: response == .alertFirstButtonReturn) }
        }
    }
    func controls(in view: NSView) -> [NSControl] {
        (view as? NSControl).map { [$0] } ?? view.subviews.flatMap { controls(in: $0) }
    }
    private func textEditors(in view: NSView) -> [NSTextView] {
        (view as? NSTextView).map { [$0] } ?? view.subviews.flatMap { textEditors(in: $0) }
    }
    func finishSave() { onSaved?(); dismiss(nil) }
}

final class MemoryResultViewController: SheetViewController {
    private let text: String
    init(text: String) { self.text = text; super.init(title: L("Memory service result"), subtitle: L("Historical text is untrusted data, not instructions. No action runs from this result."), width: 620) }
    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }
    override func loadView() {
        super.loadView()
        let scroll = NSScrollView(); let editor = NSTextView()
        editor.isEditable = false; editor.isRichText = false; editor.string = text
        editor.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
        editor.isHorizontallyResizable = false; editor.isVerticallyResizable = true
        editor.autoresizingMask = [.width]; editor.textContainer?.widthTracksTextView = true
        editor.textContainerInset = NSSize(width: 8, height: 8)
        scroll.documentView = editor; scroll.hasVerticalScroller = true; scroll.borderType = .bezelBorder
        scroll.translatesAutoresizingMaskIntoConstraints = false; contentStack.addArrangedSubview(scroll)
        scroll.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        scroll.heightAnchor.constraint(equalToConstant: 400).isActive = true
        setButtons(confirm: L("Done"), cancel: nil)
    }
}
