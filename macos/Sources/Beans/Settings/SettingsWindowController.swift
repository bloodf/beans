import AppKit

/// Settings while onboarding is up. There is no main window to hold the panes yet, and restoring
/// or pairing may need the relay URL first, so the two panes that work without an account get a
/// window of their own.
final class SettingsWindowController: NSWindowController {
    init() {
        let tabController = NSTabViewController()
        tabController.tabStyle = .toolbar

        for pane in [SettingsPane.general, .advanced] {
            let controller: NSViewController =
                pane == .general ? GeneralSettingsViewController() : AdvancedSettingsViewController()
            controller.preferredContentSize = NSSize(width: 560, height: 420)
            let item = NSTabViewItem(viewController: controller)
            item.label = pane.title
            item.image = NSImage(systemSymbolName: pane.symbolName, accessibilityDescription: pane.title)
            tabController.addTabViewItem(item)
        }

        let window = NSWindow(contentViewController: tabController)
        window.title = L("Settings")
        window.styleMask.insert(.closable)
        window.styleMask.remove(.resizable)
        window.setContentSize(NSSize(width: 560, height: 420))
        window.center()
        super.init(window: window)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }
}

// MARK: - Base

/// A settings page in the main window's content area: section cards and footnotes in one
/// scrolling column. The window's titlebar carries the page title.
class SettingsPaneViewController: NSViewController {
    let column = Build.stack([], spacing: 22)
    private let scrollView = NSScrollView()
    private let inset: CGFloat = 28

    override func loadView() {
        let container = BackgroundView()
        container.fillColor = Theme.transcriptBackground
        container.cornerRadius = 0

        column.orientation = .vertical
        column.alignment = .leading
        column.edgeInsets = NSEdgeInsets(top: 24, left: inset, bottom: 32, right: inset)

        let documentView = FlippedView()
        documentView.translatesAutoresizingMaskIntoConstraints = false
        documentView.addSubview(column)

        scrollView.documentView = documentView
        scrollView.drawsBackground = false
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.translatesAutoresizingMaskIntoConstraints = false

        // The page runs under the titlebar, which gives it AppKit's scroll-edge effect there.
        container.addSubview(scrollView)
        scrollView.pin(to: container)

        NSLayoutConstraint.activate([
            documentView.widthAnchor.constraint(equalTo: scrollView.widthAnchor),
            column.topAnchor.constraint(equalTo: documentView.topAnchor),
            column.leadingAnchor.constraint(equalTo: documentView.leadingAnchor),
            column.trailingAnchor.constraint(equalTo: documentView.trailingAnchor),
            column.bottomAnchor.constraint(equalTo: documentView.bottomAnchor),
        ])
        view = container
    }

    func add(_ row: NSView) {
        column.addArrangedSubview(row)
        row.widthAnchor.constraint(equalTo: column.widthAnchor, constant: -2 * inset).isActive = true
    }

    func addSection(_ section: SectionView) {
        section.style = .heading
        add(section)
    }

    func addFootnote(_ text: String) {
        add(Build.label(text, font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0))
    }

    /// Back to the top, which rests below the titlebar the scroll view runs under.
    func scrollToTop() {
        guard isViewLoaded else { return }
        scrollView.contentView.scroll(to: NSPoint(x: 0, y: -scrollView.contentInsets.top))
        scrollView.reflectScrolledClipView(scrollView.contentView)
    }

    /// Scrolls a setting picked from the sidebar's search into view and flashes its row. The page
    /// may have entered the window on this same click, so the layout settles first.
    func reveal(_ entry: SettingsEntry) {
        loadViewIfNeeded()
        DispatchQueue.main.async { [self] in
            view.layoutSubtreeIfNeeded()
            let sections = column.arrangedSubviews.compactMap { $0 as? SectionView }
            guard let target = sections.lazy.compactMap({ $0.target(labelled: entry.row) }).first,
                let documentView = scrollView.documentView
            else { return }
            // Room above the row for the section's title, and for the titlebar the page runs under.
            let frame = documentView.convert(target.bounds, from: target).insetBy(dx: 0, dy: -40)
            NSAnimationContext.runAnimationGroup { context in
                context.duration = 0.25
                context.allowsImplicitAnimation = true
                documentView.scrollToVisible(frame)
            }
            FlashView.flash(target)
        }
    }
}

// MARK: - General

final class GeneralSettingsViewController: SettingsPaneViewController {
    private let store = AppStore.shared
    private let pauseSwitch = NSSwitch()
    private let appearance = SettingsPopUpButton()
    private let appLanguage = SettingsPopUpButton()
    private let dictationLanguage = SettingsPopUpButton()
    private lazy var version = UpdateStatusRow(
        key: SettingsEntry.version.row, actionTitle: L("Check for Updates…"))
    private lazy var automaticDownloads = toggle(
        Updater.shared.automaticallyDownloadsUpdates, #selector(toggleAutomaticDownloads))

    override func viewDidLoad() {
        super.viewDidLoad()
        title = L("General")
        pauseSwitch.controlSize = .small
        pauseSwitch.target = self
        pauseSwitch.action = #selector(togglePause)
        let account = SectionView(title: L("Account"))
        account.setRows([
            AccessoryRow(key: L("Pause all bots"), accessory: pauseSwitch),
            NoteRow(text: L("Pausing stops new bot turns and routines on every paired Device.")),
            AccessoryRow(key: L("Memory connections"), accessory: MemoryActionButton("Manage…") { [weak self] in
                self?.presentAsSheet(MemorySettingsViewController())
            }),
        ])
        addSection(account)
        store.observe(self) { [weak self] event in
            switch event {
            case .rosterChanged, .snapshotReplaced, .connectionChanged: self?.refreshPause()
            default: break
            }
        }
        refreshPause()

        let chats = SectionView(title: L("Chats"))
        chats.setRows([
            AccessoryRow(
                key: SettingsEntry.sendOnReturn.row,
                accessory: toggle(Preferences.sendOnReturn, #selector(toggleSendOnReturn))),
            AccessoryRow(
                key: SettingsEntry.timestamps.row,
                accessory: toggle(Preferences.showTimestamps, #selector(toggleTimestamps))),
        ])
        addSection(chats)

        // In the order of `Preferences.Appearance.allCases`.
        appearance.addItems(withTitles: [L("System"), L("Light"), L("Dark")])
        appearance.selectItem(at: Preferences.Appearance.allCases.firstIndex(of: Preferences.appearance) ?? 0)
        configure(appearance, action: #selector(changeAppearance))
        let look = SectionView(title: L("Appearance"))
        // The app's own language. Each one is named in itself, so it reads whatever is showing.
        appLanguage.addItem(withTitle: L("System"))
        appLanguage.menu?.addItem(.separator())
        for (code, name) in [("en", "English"), ("zh-Hans", "简体中文")] {
            appLanguage.addItem(withTitle: name)
            appLanguage.lastItem?.representedObject = code
        }
        if let chosen = AppLanguage.chosen,
            let item = appLanguage.itemArray.first(where: { $0.representedObject as? String == chosen })
        {
            appLanguage.select(item)
        }
        configure(appLanguage, action: #selector(changeAppLanguage))
        look.setRows([
            AccessoryRow(key: SettingsEntry.appearance.row, accessory: appearance),
            AccessoryRow(key: SettingsEntry.appLanguage.row, accessory: appLanguage),
        ])
        addSection(look)

        // The language the Dictate button listens in. Automatic follows the keyboard input
        // source, then the system languages.
        dictationLanguage.addItem(withTitle: L("Automatic (%@)", Dictation.displayName(Dictation.automaticLocale())))
        dictationLanguage.menu?.addItem(.separator())
        for locale in Dictation.supportedLocales {
            dictationLanguage.addItem(withTitle: Dictation.displayName(locale))
            dictationLanguage.lastItem?.representedObject = locale.identifier
        }
        if let chosen = Preferences.dictationLanguage,
            let item = dictationLanguage.itemArray.first(where: { $0.representedObject as? String == chosen })
        {
            dictationLanguage.select(item)
        }
        configure(dictationLanguage, action: #selector(changeDictationLanguage))
        let dictation = SectionView(title: L("Dictation"))
        dictation.setRows([AccessoryRow(key: SettingsEntry.dictationLanguage.row, accessory: dictationLanguage)])
        addSection(dictation)

        if Updater.isEnabled {
            version.onAction = { Updater.shared.checkForUpdates() }
            automaticDownloads.isEnabled = Updater.shared.automaticallyChecksForUpdates
            let updates = SectionView(title: L("Updates"))
            updates.setRows([
                version,
                AccessoryRow(
                    key: SettingsEntry.automaticChecks.row,
                    accessory: toggle(Updater.shared.automaticallyChecksForUpdates, #selector(toggleAutomaticChecks))),
                AccessoryRow(key: SettingsEntry.automaticDownloads.row, accessory: automaticDownloads),
            ])
            addSection(updates)
            refreshVersion()
            NotificationCenter.default.addObserver(
                self, selector: #selector(refreshVersion), name: Updater.didFinishCheck, object: nil)
        }

        addFootnote(
            L("Appearance, chats, and dictation are local to this computer. Pause is shared with every paired Device.")
        )
    }

    private func refreshPause() {
        pauseSwitch.state = store.paused ? .on : .off
        pauseSwitch.isEnabled = store.hasIdentity == true && (store.isMock || store.isConnected)
    }

    @objc private func togglePause(_ sender: NSSwitch) {
        let desired = sender.state == .on
        sender.isEnabled = false
        Task { [weak self] in
            guard let self else { return }
            do { try await store.setPaused(desired) }
            catch {
                let alert = NSAlert()
                alert.messageText = L("Couldn’t change account pause")
                alert.informativeText = error.localizedDescription
                alert.addButton(withTitle: L("OK"))
                if let window = view.window { alert.beginSheetModal(for: window) { _ in } }
            }
            refreshPause()
        }
    }

    private func toggle(_ isOn: Bool, _ action: Selector) -> NSSwitch {
        let toggle = NSSwitch()
        toggle.controlSize = .small
        toggle.state = isOn ? .on : .off
        toggle.target = self
        toggle.action = action
        return toggle
    }

    private func configure(_ popUp: NSPopUpButton, action: Selector) {
        popUp.target = self
        popUp.action = action
    }

    @objc private func toggleSendOnReturn(_ sender: NSSwitch) {
        Preferences.sendOnReturn = sender.state == .on
    }

    @objc private func toggleTimestamps(_ sender: NSSwitch) {
        Preferences.showTimestamps = sender.state == .on
    }

    @objc private func toggleAutomaticChecks(_ sender: NSSwitch) {
        Updater.shared.automaticallyChecksForUpdates = sender.state == .on
        automaticDownloads.isEnabled = sender.state == .on
    }

    @objc private func toggleAutomaticDownloads(_ sender: NSSwitch) {
        Updater.shared.automaticallyDownloadsUpdates = sender.state == .on
    }

    @objc private func refreshVersion() {
        version.setValue("\(Updater.currentVersion) · \(Updater.shared.lastCheckDescription)")
    }

    @objc private func changeDictationLanguage() {
        Preferences.dictationLanguage = dictationLanguage.selectedItem?.representedObject as? String
    }

    /// The app says everything again in the new language: `AppDelegate` rebuilds the menu, and
    /// the window builds its panes again in place, this one included.
    @objc private func changeAppLanguage() {
        AppLanguage.choose(appLanguage.selectedItem?.representedObject as? String)
    }

    @objc private func changeAppearance() {
        Preferences.appearance = Preferences.Appearance.allCases[max(0, appearance.indexOfSelectedItem)]
        NSApp.appearance = Preferences.appearance.nsAppearance
    }
}

// MARK: - Advanced

final class AdvancedSettingsViewController: SettingsPaneViewController {
    private let relay = EditableRow(key: SettingsEntry.relayURL.row, placeholder: AppInfo.productionRelayURL)
    private let port = EditableRow(key: SettingsEntry.cliPort.row, placeholder: String(AppInfo.defaultCLIPort))

    override func viewDidLoad() {
        super.viewDidLoad()
        title = L("Advanced")

        relay.field.stringValue = AppStore.shared.relayURL ?? Preferences.relayURL
        port.field.stringValue = "\(Preferences.cliPort)"
        for row in [relay, port] {
            row.field.font = .monospacedSystemFont(ofSize: 11, weight: .regular)
            row.onCommit = { [weak self] in self?.commit() }
        }
        let connection = SectionView(title: L("Connection"))
        connection.setRows([relay, port])
        addSection(connection)
        addFootnote(
            L("Self-hosting the relay is a URL change: clients sign their requests and upload ciphertext, so the relay has nothing to trust. Leave it empty to use the default relay.")
        )

        let onboarding = ActionRow(
            key: SettingsEntry.onboarding.row, value: "", tint: .secondaryLabelColor, actionTitle: L("Show Onboarding Again"))
        // Onboarding closes the window this row is in, so the click returns first.
        onboarding.onAction = {
            DispatchQueue.main.async {
                NSApp.sendAction(#selector(AppDelegate.showOnboarding(_:)), to: nil, from: nil)
            }
        }
        let setup = SectionView(title: L("Setup"))
        setup.setRows([onboarding])
        addSection(setup)

        guard AppStore.shared.hasIdentity == true else { return }
        let delete = ActionRow(
            key: SettingsEntry.deleteAccount.row, value: "", tint: .secondaryLabelColor, actionTitle: L("Delete Account…"))
        delete.onAction = { [weak self] in self?.confirmDeleteAccount() }
        let account = SectionView(title: L("Account"))
        account.setRows([delete])
        addSection(account)
        addFootnote(L("Deletes the account from the relay and from every paired Device: chats, attachments, bots, and provider credentials."))
    }

    private func confirmDeleteAccount() {
        guard let window = view.window else { return }
        let alert = NSAlert()
        alert.messageText = L("Delete this account?")
        alert.informativeText = L("The relay deletes everything it holds for the account, and every paired Device, this one included, forgets its keys and chats. This can’t be undone.")
        alert.alertStyle = .critical
        alert.addButton(withTitle: L("Delete Account"))
        alert.addButton(withTitle: L("Cancel"))
        alert.buttons.first?.hasDestructiveAction = true
        alert.beginSheetModal(for: window) { response in
            guard response == .alertFirstButtonReturn else { return }
            Task { @MainActor in
                do {
                    try await AppStore.shared.deleteAccount()
                } catch {
                    let failed = NSAlert()
                    failed.messageText = L("Couldn’t delete the account")
                    failed.informativeText = error.localizedDescription
                    failed.addButton(withTitle: L("OK"))
                    failed.beginSheetModal(for: window) { _ in }
                }
            }
        }
    }

    private func commit() {
        if relay.value != (AppStore.shared.relayURL ?? "") {
            Preferences.relayURL = relay.value
            AppStore.shared.setRelayURL(relay.value)
        }
        if let number = Int(port.value), number == AppInfo.defaultCLIPort {
            if number != Preferences.cliPort {
                Preferences.cliPort = number
                AppStore.shared.reconnect()
            }
        } else {
            port.field.stringValue = "\(Preferences.cliPort)"
        }
    }
}
