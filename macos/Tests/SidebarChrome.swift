import AppKit

// Only the sidebar's domain inputs are doubled. Search, pane cells, outlines, scroll views,
// footer controls and layout execute production code. No account, CLI or avatar is loaded.
func L(_ text: String, _ args: CVarArg...) -> String { String(format: text, arguments: args) }
struct Device {
    typealias ID = String
    enum Status { case online, pairing, offline }
    enum OS { case macos, linux, windows, ios, ipados, android, unknown }
    var id = "fixture-device"
    var name = "Fixture Mac"
    var os: OS = .macos
    var model = "MacBook"
}
enum SettingsPane: String, CaseIterable {
    case general, providers, autoReview, plugins, bots, device, advanced
}
enum Selection: Equatable { case chat(String), settings(SettingsPane) }
struct SettingsEntry: Hashable { var pane: SettingsPane; var title: String }
enum SettingsSearch {
    struct PaneResult { var pane: SettingsPane; var entries: [SettingsEntry] }
    static func panes(matching query: String, device: Device?, store: AppStore) -> [PaneResult] {
        SettingsPane.allCases.filter { $0.title.localizedCaseInsensitiveContains(query) }
            .map { PaneResult(pane: $0, entries: []) }
    }
}
struct Chat {
    typealias ID = String
    var id: ID
    var isPinned = false
    var isGroup = false
    var isDM = true
}
@MainActor
final class AppStore {
    static let shared = AppStore()
    enum Event {
        case chatsChanged, chatChanged(String), respondingChanged(String), messageChanged(String, String)
        case messageRemoved(String, String), snapshotReplaced, connectionChanged, rosterChanged
    }
    var chats = [Chat(id: "fixture-chat")]
    var thisDevice: Device? = Device()
    var paused = false
    var isConnected = true
    var relayError: String?
    func chat(_ id: String) -> Chat? { chats.first { $0.id == id } }
    func device(_ id: String) -> Device? { thisDevice }
    func observe(_ owner: AnyObject, _ callback: @escaping (Event) -> Void) {}
}
// Chat content is outside these checks; the production outline still determines first-row geometry.
final class SidebarChatCell: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("FixtureChatCell")
    struct Content { init(chat: Chat, store: AppStore) {} }
    var shortcutNumber: Int?
    func configure(_ content: Content) {}
}
final class AppDelegate: NSObject {
    @objc func toggleCommandPalette(_ sender: Any?) {}
}
final class RootSplitViewController: NSObject {
    @objc func togglePinChat(_ sender: Any?) {}
    @objc func renameChat(_ sender: Any?) {}
    @objc func addBotToChat(_ sender: Any?) {}
    @objc func deleteChat(_ sender: Any?) {}
}
enum AppInfo { static let cliCommand = "fixture only" }
enum Preferences { static let cliPort = 0 }

@main
struct SidebarChromeChecks {
    @MainActor static func main() {
        _ = NSApplication.shared
        if ProcessInfo.processInfo.environment["BEANS_CHROME_SHOW"] == "1" {
            let fixture = SidebarChromeWindow()
            NSApp.delegate = fixture
            fixture.showWindow(nil)
            print("READY: isolated sidebar window")
            fflush(stdout)
            NSApp.run()
            return
        }
        let failures = NativeChromeRenderingChecks.runSidebar()
        failures.forEach { print("FAIL: \($0)") }
        print("\(failures.isEmpty ? "PASS" : "FAIL"): sidebar chrome (\(failures.count) failures)")
        if !failures.isEmpty { exit(1) }
    }
}

/// A visible host for bounded screenshot/AX acceptance of the production controls.
/// The content pane and domain inputs are fixtures, not an app or provider integration.
@MainActor
final class SidebarChromeWindow: NSWindowController, NSApplicationDelegate {
    private let split = NSSplitViewController()
    private let host = NSViewController()
    private let chats = SidebarViewController()
    private let settings = SettingsSidebarViewController()
    private var item: NSSplitViewItem!
    private let popup = SettingsPopUpButton()

    init() {
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1180, height: 600),
            styleMask: [.titled, .closable, .resizable, .fullSizeContentView], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.title = "Phase 1 Sidebar Fixture"
        window.toolbar = NSToolbar(identifier: "SidebarVisibleFixture")
        window.toolbarStyle = .unified
        window.appearance = NSAppearance(named: .darkAqua)
        super.init(window: window)
        host.view = NSView()
        item = NSSplitViewItem(sidebarWithViewController: host)
        item.minimumThickness = 232
        item.maximumThickness = 340
        split.addSplitViewItem(item)
        let content = NSViewController()
        content.view = NSView()
        popup.addItems(withTitles: ["Fixture local choice", "Second local choice"])
        let row = AccessoryRow(key: "Fixture popup", accessory: popup)
        content.view.addSubview(row)
        NSLayoutConstraint.activate([
            row.leadingAnchor.constraint(equalTo: content.view.leadingAnchor, constant: 24),
            row.trailingAnchor.constraint(equalTo: content.view.trailingAnchor, constant: -24),
            row.topAnchor.constraint(equalTo: content.view.safeAreaLayoutGuide.topAnchor, constant: 24),
        ])
        split.addSplitViewItem(NSSplitViewItem(viewController: content))
        split.view.frame = window.contentView!.frame
        window.contentViewController = split
        chats.footer.onSettings = { [weak self] in self?.showSettings(true) }
        settings.onBack = { [weak self] in self?.showSettings(false) }
        settings.onSelect = { [weak self] selection in
            self?.settings.setSelection(selection)
        }
        showSettings(true)
        split.view.layoutSubtreeIfNeeded()
        split.splitView.setPosition(260, ofDividerAt: 0)
        window.center()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    private func showSettings(_ enabled: Bool) {
        host.children.forEach { $0.view.removeFromSuperview(); $0.removeFromParent() }
        let controller: NSViewController = enabled ? settings : chats
        host.addChild(controller)
        host.view.addSubview(controller.view)
        controller.view.pin(to: host.view)
        let topBar = enabled ? settings.header : chats.searchBar
        let bottomBar: NSView = enabled ? settings.footer : chats.footer
        if #available(macOS 26.0, *) {
            let top = NSSplitViewItemAccessoryViewController()
            top.view = topBar
            top.automaticallyAppliesContentInsets = true
            let bottom = NSSplitViewItemAccessoryViewController()
            bottom.view = bottomBar
            bottom.automaticallyAppliesContentInsets = true
            item.topAlignedAccessoryViewControllers = [top]
            item.bottomAlignedAccessoryViewControllers = [bottom]
        }
        if enabled {
            settings.resetSearch()
            settings.setSelection(.settings(.general))
            settings.focusList()
        }
        window?.contentView?.layoutSubtreeIfNeeded()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
}
