import AppKit
import XCTest
@testable import Beans

/// Real `ComposerView`, `MentionPanel` and key routing: events go through `NSApp.sendEvent`
/// into the composer's own text view, and the only observable outputs are `onSend` and the
/// field text. Reads `AppStore.shared` only through the picker's runner lookup; sets nothing.
@MainActor
final class ComposerReturnTests: XCTestCase {
    private var window: NSWindow!
    private var composer: ComposerView!
    private var textView: ComposerTextView!
    private var sent: [(text: String, files: Int)] = []
    private var savedSendOnReturn: Bool!
    private let bots = ["Ada", "Bea"].map {
        Bot(
            id: "composer-test-\($0)", name: $0, description: "", symbolName: "sparkles", accent: .indigo,
            runnerID: "composer-test-runner", provider: .deepseek, createdAt: Date(timeIntervalSince1970: 0))
    }

    override func setUp() async throws {
        _ = NSApplication.shared
        savedSendOnReturn = Preferences.sendOnReturn
        composer = ComposerView()
        composer.frame = NSRect(x: 0, y: 0, width: 600, height: 160)
        composer.configure(placeholder: "Message", bots: bots)
        composer.onSend = { [unowned self] text, files, _, _ in sent.append((text, files.count)) }
        window = NSWindow(contentRect: composer.frame, styleMask: [.titled], backing: .buffered, defer: false)
        window.contentView = composer
        window.makeKeyAndOrderFront(nil)
        composer.focus()
        textView = try XCTUnwrap(find(ComposerTextView.self, in: composer))
        XCTAssertTrue(window.firstResponder === textView)
    }

    override func tearDown() async throws {
        Preferences.sendOnReturn = savedSendOnReturn
        window.orderOut(nil)
    }

    private func find<T: NSView>(_ type: T.Type, in view: NSView) -> T? {
        if let match = view as? T { return match }
        for sub in view.subviews { if let match = find(type, in: sub) { return match } }
        if let scroll = view as? NSScrollView, let match = scroll.documentView as? T { return match }
        return nil
    }

    /// Dispatches as the app does, so `NSApp.currentEvent` and `keyDown` both see the event.
    private func press(_ flags: NSEvent.ModifierFlags = [], keyCode: UInt16 = 36) {
        let character = keyCode == 48 ? "\t" : keyCode == 76 ? "\u{3}" : "\r"
        let event = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: flags, timestamp: 0, windowNumber: window.windowNumber,
            context: nil, characters: character, charactersIgnoringModifiers: character, isARepeat: false,
            keyCode: keyCode)!
        NSApp.postEvent(event, atStart: true)
        if let next = NSApp.nextEvent(matching: .any, until: Date(timeIntervalSinceNow: 0.5), inMode: .default, dequeue: true) {
            NSApp.sendEvent(next)
        }
    }

    private func type(_ text: String) {
        composer.text = ""
        textView.insertText(text, replacementRange: NSRange(location: NSNotFound, length: 0))
    }

    func testSendOnReturnOffSendsOnlyOnCommandReturn() {
        Preferences.sendOnReturn = false
        type("hello")
        press()
        XCTAssertEqual(sent.count, 0)
        XCTAssertEqual(composer.text, "hello\n")
        press(.shift)
        XCTAssertEqual(sent.count, 0)
        press([.command, .shift])
        XCTAssertEqual(sent.count, 0)
        press(.option)
        press(.control)
        XCTAssertEqual(sent.count, 0)
        press(.command)
        XCTAssertEqual(sent.map(\.text), ["hello"])
        XCTAssertEqual(composer.text, "")
    }

    func testCommandEnterOnKeypadSendsOnce() {
        Preferences.sendOnReturn = false
        type("hi")
        press(.command, keyCode: 76)
        XCTAssertEqual(sent.map(\.text), ["hi"])
    }

    func testSendOnReturnOnSendsReturnAndBreaksOnShift() {
        Preferences.sendOnReturn = true
        type("hello")
        press(.shift)
        XCTAssertEqual(sent.count, 0)
        XCTAssertEqual(composer.text, "hello\n")
        press([.command, .shift])
        XCTAssertEqual(sent.count, 0)
        press()
        XCTAssertEqual(sent.map(\.text), ["hello"])
        type("again")
        press(.command)
        XCTAssertEqual(sent.map(\.text), ["hello", "again"])
    }

    func testMarkedTextNeverSends() {
        for on in [false, true] {
            Preferences.sendOnReturn = on
            type("")
            textView.setMarkedText(
                "に", selectedRange: NSRange(location: 1, length: 0), replacementRange: NSRange(location: NSNotFound, length: 0))
            press()
            press(.command)
            XCTAssertEqual(sent.count, 0, "sendOnReturn=\(on)")
            XCTAssertTrue(textView.hasMarkedText(), "sendOnReturn=\(on)")
            textView.unmarkText()
        }
    }

    func testMentionPickerKeepsSelectionPriority() {
        for (on, flags, keyCode): (Bool, NSEvent.ModifierFlags, UInt16) in [
            (true, [], 36), (false, [], 36), (false, .command, 36), (true, .command, 36), (true, [], 48),
        ] {
            Preferences.sendOnReturn = on
            type("hey @A")
            press(flags, keyCode: keyCode)
            XCTAssertEqual(sent.count, 0, "on=\(on) flags=\(flags.rawValue) key=\(keyCode)")
            XCTAssertEqual(composer.text, "hey @Ada ", "on=\(on) flags=\(flags.rawValue) key=\(keyCode)")
        }
    }

    func testNewlineKeysKeepAttachmentsAndSendConsumesThem() throws {
        Preferences.sendOnReturn = false
        let file = FileManager.default.temporaryDirectory.appendingPathComponent("composer-return-\(UUID().uuidString).txt")
        try Data("x".utf8).write(to: file)
        defer { try? FileManager.default.removeItem(at: file) }
        XCTAssertTrue(composer.addFiles([file]))
        type("with file")
        press()
        press(.shift)
        XCTAssertEqual(composer.attachments.count, 1)
        XCTAssertEqual(sent.count, 0)
        press(.command)
        XCTAssertEqual(sent.map(\.files), [1])
        XCTAssertTrue(composer.attachments.isEmpty)
    }
}
