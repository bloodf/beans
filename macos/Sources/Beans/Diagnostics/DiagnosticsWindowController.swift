import AppKit
import UniformTypeIdentifiers

@MainActor
final class DiagnosticsWindowController: NSWindowController {
    private let review: DiagnosticsViewController

    init(client: CLIClient) {
        review = DiagnosticsViewController(client: client)
        let window = NSWindow(contentViewController: review)
        window.title = L("Diagnostics")
        window.styleMask = [.titled, .closable, .miniaturizable, .resizable]
        window.setContentSize(NSSize(width: 720, height: 620))
        window.minSize = NSSize(width: 520, height: 420)
        window.center()
        super.init(window: window)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func languageChanged() {
        window?.title = L("Diagnostics")
        review.localize()
    }
}

@MainActor
private final class DiagnosticsViewController: NSViewController {
    private enum State { case idle, loading, ready, unsupported, failed, copied, exportFailed }
    private let client: CLIClient
    private var state: State = .idle
    private var report: DiagnosticsReport?
    private var payload: String?
    private let disclosure = NSTextField(wrappingLabelWithString: "")
    private let limits = NSTextField(wrappingLabelWithString: "")
    private let status = NSTextField(wrappingLabelWithString: "")
    private let text = NSTextView()
    private let refresh = NSButton()
    private let copy = NSButton()
    private let export = NSButton()

    init(client: CLIClient) {
        self.client = client
        super.init(nibName: nil, bundle: nil)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        view = NSView()
        let scroll = NSScrollView()
        scroll.hasVerticalScroller = true
        scroll.borderType = .bezelBorder
        text.isEditable = false
        text.isSelectable = true
        text.isRichText = false
        text.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
        text.textContainerInset = NSSize(width: 10, height: 10)
        text.isVerticallyResizable = true
        text.isHorizontallyResizable = false
        text.autoresizingMask = [.width]
        text.textContainer?.widthTracksTextView = true
        scroll.documentView = text

        for (button, action, key) in [
            (refresh, #selector(fetchReport), "r"),
            (copy, #selector(copyReport), "c"),
            (export, #selector(exportReport), "s")
        ] {
            button.bezelStyle = .rounded
            button.target = self
            button.action = action
            button.keyEquivalent = key
            button.keyEquivalentModifierMask = [.command, .shift]
        }
        let buttons = NSStackView(views: [refresh, copy, export])
        buttons.spacing = 10
        let column = NSStackView(views: [disclosure, limits, status, scroll, buttons])
        column.orientation = .vertical
        column.alignment = .leading
        column.spacing = 12
        column.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(column)
        NSLayoutConstraint.activate([
            column.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 20),
            column.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -20),
            column.topAnchor.constraint(equalTo: view.topAnchor, constant: 20),
            column.bottomAnchor.constraint(equalTo: view.bottomAnchor, constant: -20),
            scroll.widthAnchor.constraint(equalTo: column.widthAnchor),
            scroll.heightAnchor.constraint(greaterThanOrEqualToConstant: 140),
            disclosure.widthAnchor.constraint(equalTo: column.widthAnchor),
            limits.widthAnchor.constraint(equalTo: column.widthAnchor),
            status.widthAnchor.constraint(equalTo: column.widthAnchor)
        ])
        text.nextKeyView = refresh
        refresh.nextKeyView = copy
        copy.nextKeyView = export
        export.nextKeyView = text
        localize()
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        if case .idle = state { fetchReport() }
    }

    func localize() {
        guard isViewLoaded else { return }
        disclosure.stringValue = report.map {
            L("Relay host/port disclosed in this report: %@", $0.relay.host ?? L("Not available"))
        } ?? L("This report discloses the selected relay host and explicit port. Review before sharing. Nothing uploads automatically.")
        limits.stringValue = L("Providers: configured-only, authentication not checked. Plugins: cache-only anonymous setup slots. Runners: presence-only, not admission readiness. A busy local port is a note, not proof of failure.")
        refresh.title = L("Refresh")
        copy.title = L("Copy Report")
        export.title = L("Export Report…")
        text.setAccessibilityLabel(L("Diagnostics report JSON"))
        for button in [refresh, copy, export] { button.setAccessibilityLabel(button.title) }
        switch state {
        case .idle: status.stringValue = L("Open diagnostics to request a local report.")
        case .loading: status.stringValue = L("Loading diagnostics…")
        case .ready: status.stringValue = L("Review the report below before copying or exporting it.")
        case .unsupported: status.stringValue = L("Unsupported diagnostics schema. Update Beans before sharing a report.")
        case .failed: status.stringValue = L("Diagnostics unavailable. No report was copied or exported. Check the local CLI and try again.")
        case .copied: status.stringValue = L("Report copied to the clipboard.")
        case .exportFailed: status.stringValue = L("Could not export the report. Choose a writable destination and try again.")
        }
        refresh.isEnabled = !isLoading
        copy.isEnabled = payload != nil && !isLoading
        export.isEnabled = payload != nil && !isLoading
    }

    private var isLoading: Bool {
        if case .loading = state { return true }
        return false
    }

    @objc private func fetchReport() {
        guard !isLoading else { return }
        report = nil
        payload = nil
        text.string = ""
        state = .loading
        localize()
        Task { [weak self] in
            guard let self else { return }
            do {
                let data = try await client.request("diagnostics.report")
                let decoded = try DiagnosticsReport.read(data)
                let json = try decoded.json()
                report = decoded
                payload = json
                text.string = json
                state = .ready
            } catch DiagnosticsReport.ReportError.unsupportedSchema {
                state = .unsupported
            } catch {
                // Local/remote errors may contain private paths or values. Never display/export them.
                state = .failed
            }
            localize()
        }
    }

    @objc private func copyReport() {
        guard let payload, !isLoading else { return }
        NSPasteboard.general.clearContents()
        if NSPasteboard.general.setString(payload, forType: .string) { state = .copied }
        localize()
    }

    @objc private func exportReport() {
        guard let payload, !isLoading, let window = view.window else { return }
        let panel = NSSavePanel()
        panel.title = L("Export Report…")
        panel.allowedContentTypes = [.json]
        panel.nameFieldStringValue = "beans-diagnostics.json"
        panel.beginSheetModal(for: window) { [weak self] response in
            guard response == .OK, let url = panel.url else { return }
            do {
                try payload.write(to: url, atomically: true, encoding: .utf8)
            } catch {
                self?.state = .exportFailed
                self?.localize()
            }
        }
    }
}
