import AppKit
import XCTest
@testable import Beans

/// Real `ComposerView`, `MentionPanel` and key routing: events go through `NSApp.sendEvent`
/// into the composer's own text view, and the observable outputs are `onSend`, the field text
/// and attachments. Reads `AppStore.shared` only through the picker's runner lookup. The
/// send-on-Return preference is overridden in `UserDefaults.argumentDomain`, a volatile domain
/// that outranks the stored value and never reaches disk, so nothing persistent is written
/// even if the test process dies. `Preferences.Key` is private, hence the literal key.
@MainActor
final class ComposerReturnTests: XCTestCase {
    private var window: NSWindow!
    private var composer: ComposerView!
    private var textView: ComposerTextView!
    private var sent: [(text: String, files: Int)] = []
    private static let sendOnReturnKey = "beans-v2.sendOnReturn"
    /// The whole argument domain as it was, restored in `tearDown`.
    private var savedArguments: [String: Any] = [:]
    private let bots = ["Ada", "Bea"].map {
        Bot(
            id: "composer-test-\($0)", name: $0, description: "", symbolName: "sparkles", accent: .indigo,
            runnerID: "composer-test-runner", provider: .deepseek, createdAt: Date(timeIntervalSince1970: 0))
    }

    override func setUp() async throws {
        _ = NSApplication.shared
        // XCTest is not the app bundle: point the picker's avatars at the committed JSC resource
        // the packaging step copies, the same way the other native fixtures do.
        if AvatarGeometryBridge.scriptURL == nil {
            let url = URL(fileURLWithPath: #filePath)
                .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
                .deletingLastPathComponent().appendingPathComponent("packages/beans-blobatar/dist/blobatar.jsc.js")
            _ = try url.checkResourceIsReachable()
            AvatarGeometryBridge.scriptURL = url
        }
        savedArguments = UserDefaults.standard.volatileDomain(forName: UserDefaults.argumentDomain)
        sent = []
        composer = ComposerView()
        composer.frame = NSRect(x: 0, y: 0, width: 600, height: 160)
        composer.configure(placeholder: "Message", bots: bots)
        composer.onSend = { [unowned self] text, files, _, _ in sent.append((text, files.count)) }
        window = NSWindow(contentRect: composer.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = composer
        // A hosted test process need not be the active app, so the window is not required to be
        // key; `press` sends to this window directly and checks the first responder instead.
        window.orderFront(nil)
        XCTAssertTrue(window.makeFirstResponder(textView), "text view refused first responder")
        textView = try XCTUnwrap(find(ComposerTextView.self, in: composer))
        XCTAssertTrue(window.firstResponder === textView)
    }

    override func tearDown() async throws {
        // Empty text dismisses the picker; then drop any child panel and the window itself.
        composer.clearDraft()
        for child in window.childWindows ?? [] {
            window.removeChildWindow(child)
            child.orderOut(nil)
        }
        window.contentView = nil
        window.orderOut(nil)
        window.close()
        textView = nil
        composer = nil
        window = nil
        UserDefaults.standard.setVolatileDomain(savedArguments, forName: UserDefaults.argumentDomain)
    }

    private func setSendOnReturn(_ value: Bool) {
        var arguments = savedArguments
        arguments[Self.sendOnReturnKey] = value
        UserDefaults.standard.setVolatileDomain(arguments, forName: UserDefaults.argumentDomain)
        XCTAssertEqual(Preferences.sendOnReturn, value, "argument-domain override not visible to Preferences")
    }

    private func find<T: NSView>(_ type: T.Type, in view: NSView) -> T? {
        if let match = view as? T { return match }
        for sub in view.subviews { if let match = find(type, in: sub) { return match } }
        if let scroll = view as? NSScrollView, let match = scroll.documentView as? T { return match }
        return nil
    }

    /// Delivers a real key event to the composer window with `window.sendEvent`, which routes
    /// to the first responder's production `keyDown` and `interpretKeyEvents` without needing
    /// the hosted test process to be the active app (so no `NSApp` queue, which would also
    /// let another event be dequeued). The existing `onKeyCommand` hook is wrapped, not
    /// replaced, to record the commands AppKit produced; that proves the key reached the text
    /// view's command path. Returns the selector names, in order.
    @discardableResult
    private func press(_ flags: NSEvent.ModifierFlags = [], keyCode: UInt16 = 36) throws -> [String] {
        XCTAssertTrue(window.firstResponder === textView, "text view is not first responder")
        let character = keyCode == 48 ? "\t" : keyCode == 76 ? "\u{3}" : "\r"
        let event = try XCTUnwrap(
            NSEvent.keyEvent(
                with: .keyDown, location: .zero, modifierFlags: flags, timestamp: ProcessInfo.processInfo.systemUptime,
                windowNumber: window.windowNumber, context: nil, characters: character,
                charactersIgnoringModifiers: character, isARepeat: false, keyCode: keyCode))
        let original = try XCTUnwrap(textView.onKeyCommand, "composer did not install its key hook")
        var commands: [String] = []
        textView.onKeyCommand = { selector in
            commands.append(NSStringFromSelector(selector))
            return original(selector)
        }
        defer { textView.onKeyCommand = original }
        window.sendEvent(event)
        return commands
    }

    private func type(_ text: String) {
        composer.text = ""
        textView.insertText(text, replacementRange: NSRange(location: NSNotFound, length: 0))
    }

    func testSendOnReturnOffSendsOnlyOnCommandReturn() throws {
        setSendOnReturn(false)
        type("hello")
        try press()
        XCTAssertEqual(sent.count, 0)
        XCTAssertEqual(composer.text, "hello\n")
        try press(.shift)
        XCTAssertEqual(sent.count, 0)
        XCTAssertEqual(composer.text, "hello\n\n")
        // Delivery of each of these is asserted by `press`; their text effect is AppKit's.
        try press([.command, .shift])
        try press(.option)
        try press(.control)
        XCTAssertEqual(sent.count, 0)
        let beforeSend = composer.text
        XCTAssertTrue(beforeSend.hasPrefix("hello\n\n"))
        try press(.command)
        XCTAssertEqual(sent.map(\.text), [beforeSend.trimmingCharacters(in: .whitespacesAndNewlines)])
        XCTAssertEqual(composer.text, "")
    }

    func testCommandEnterOnKeypadSendsOnce() throws {
        setSendOnReturn(false)
        type("hi")
        try press(.command, keyCode: 76)
        XCTAssertEqual(sent.map(\.text), ["hi"])
        XCTAssertEqual(composer.text, "")
    }

    func testSendOnReturnOnSendsReturnAndBreaksOnShift() throws {
        setSendOnReturn(true)
        type("hello")
        try press(.shift)
        XCTAssertEqual(sent.count, 0)
        XCTAssertEqual(composer.text, "hello\n")
        try press([.command, .shift])
        XCTAssertEqual(sent.count, 0)
        try press()
        XCTAssertEqual(sent.map(\.text), ["hello"])
        XCTAssertEqual(composer.text, "")
        type("again")
        try press(.command)
        XCTAssertEqual(sent.map(\.text), ["hello", "again"])
    }

    func testMarkedTextNeverSends() throws {
        for on in [false, true] {
            setSendOnReturn(on)
            for flags: NSEvent.ModifierFlags in [[], .command] {
                composer.text = ""
                textView.setMarkedText(
                    "に", selectedRange: NSRange(location: 1, length: 0),
                    replacementRange: NSRange(location: NSNotFound, length: 0))
                XCTAssertTrue(textView.hasMarkedText(), "setup on=\(on) flags=\(flags.rawValue)")
                try press(flags)
                // Pass-through may commit or keep the composition; either way no send and the
                // composed character is still in the draft.
                XCTAssertEqual(sent.count, 0, "on=\(on) flags=\(flags.rawValue)")
                XCTAssertTrue(composer.text.contains("に"), "on=\(on) flags=\(flags.rawValue) text=\(composer.text)")
                textView.unmarkText()
            }
        }
    }

    func testMentionPickerKeepsSelectionPriority() throws {
        for (on, flags, keyCode): (Bool, NSEvent.ModifierFlags, UInt16) in [
            (true, [], 36), (false, [], 36), (false, .command, 36), (true, .command, 36), (true, [], 48),
        ] {
            setSendOnReturn(on)
            type("hey @A")
            XCTAssertEqual(window.childWindows?.count, 1, "picker not showing: on=\(on) key=\(keyCode)")
            try press(flags, keyCode: keyCode)
            XCTAssertEqual(sent.count, 0, "on=\(on) flags=\(flags.rawValue) key=\(keyCode)")
            XCTAssertEqual(composer.text, "hey @Ada ", "on=\(on) flags=\(flags.rawValue) key=\(keyCode)")
            XCTAssertTrue(window.childWindows?.isEmpty ?? true, "picker still showing after pick")
        }
    }

    func testNewlineKeysKeepAttachmentsAndSendConsumesThem() throws {
        setSendOnReturn(false)
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("composer-return-\(UUID().uuidString).txt")
        try Data("x".utf8).write(to: file)
        defer { try? FileManager.default.removeItem(at: file) }
        XCTAssertTrue(composer.addFiles([file]))
        type("with file")
        try press()
        XCTAssertEqual(composer.text, "with file\n")
        try press(.shift)
        XCTAssertEqual(composer.text, "with file\n\n")
        XCTAssertEqual(composer.attachments.count, 1)
        XCTAssertEqual(sent.count, 0)
        try press(.command)
        XCTAssertEqual(sent.map(\.files), [1])
        XCTAssertTrue(composer.attachments.isEmpty)
    }
}
