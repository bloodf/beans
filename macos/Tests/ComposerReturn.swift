import AppKit

// Drives the production ComposerTextView with real key events and counts sends.
// Theme is the one dependency of the extracted text view.
enum Theme { enum Font { static let message = NSFont.systemFont(ofSize: 13) } }

@MainActor var failures = 0
@MainActor func check(_ ok: Bool, _ message: String) {
    if !ok { failures += 1; print("FAIL: \(message)") }
}

@MainActor func shouldSend(_ view: ComposerTextView, _ selector: Selector, sendOnReturn: Bool) -> Bool {
    view.returnIntent(for: selector, sendOnReturn: sendOnReturn) == .send
}

@MainActor func press(
    _ flags: NSEvent.ModifierFlags, sendOnReturn: Bool, keyCode: UInt16 = 36, marked: Bool = false
) -> (sends: Int, text: String) {
    let view = ComposerTextView(frame: NSRect(x: 0, y: 0, width: 200, height: 100))
    let window = NSWindow(contentRect: view.frame, styleMask: [.titled], backing: .buffered, defer: false)
    window.contentView = view
    window.makeFirstResponder(view)
    var sends = 0
    view.onKeyCommand = { selector in
        guard shouldSend(view, selector, sendOnReturn: sendOnReturn) else { return false }
        sends += 1
        return true
    }
    if marked {
        view.setMarkedText("に", selectedRange: NSRange(location: 1, length: 0), replacementRange: NSRange(location: NSNotFound, length: 0))
    }
    let character = keyCode == 76 ? "\u{3}" : "\r"
    let event = NSEvent.keyEvent(
        with: .keyDown, location: .zero, modifierFlags: flags, timestamp: 0, windowNumber: window.windowNumber,
        context: nil, characters: character, charactersIgnoringModifiers: character, isARepeat: false, keyCode: keyCode)!
    // Dispatch as the app does (postEvent -> nextEvent -> sendEvent) so NSApp.currentEvent is set.
    window.makeKeyAndOrderFront(nil)
    window.makeFirstResponder(view)
    NSApp.postEvent(event, atStart: true)
    guard let next = NSApp.nextEvent(matching: .any, until: Date(timeIntervalSinceNow: 0.5), inMode: .default, dequeue: true)
    else { print("no event dequeued"); return (sends, view.string) }
    NSApp.sendEvent(next)
    window.orderOut(nil)
    return (sends, view.string)
}
@main
@MainActor
struct ComposerReturnChecks {
    static func main() {
        _ = NSApplication.shared
        var r = press([], sendOnReturn: false)
        check(r.sends == 0 && r.text == "\n", "off: Return inserts newline, sends 0 (got \(r))")
        r = press(.command, sendOnReturn: false)
        check(r.sends == 1, "off: ⌘Return sends exactly once (got \(r))")
        r = press(.command, sendOnReturn: false, keyCode: 76)
        check(r.sends == 1, "off: ⌘Enter sends exactly once (got \(r))")
        r = press(.shift, sendOnReturn: false)
        check(r.sends == 0 && r.text == "\n", "off: Shift-Return inserts newline (got \(r))")
        r = press([.command, .shift], sendOnReturn: false)
        check(r.sends == 0, "off: ⌘Shift-Return never sends (got \(r))")
        r = press([], sendOnReturn: true)
        check(r.sends == 1 && r.text.isEmpty, "on: Return sends once (got \(r))")
        r = press(.shift, sendOnReturn: true)
        check(r.sends == 0 && r.text == "\n", "on: Shift-Return inserts newline (got \(r))")
        r = press([.command, .shift], sendOnReturn: true)
        check(r.sends == 0, "on: ⌘Shift-Return never sends (got \(r))")
        r = press(.command, sendOnReturn: true)
        check(r.sends == 1, "on: ⌘Return sends once (got \(r))")
        for on in [false, true] {
            for flags: NSEvent.ModifierFlags in [.option, .control, [.command, .option], [.command, .control]] {
                r = press(flags, sendOnReturn: on)
                check(r.sends == 0, "unrelated modifiers never send (on=\(on), flags=\(flags.rawValue)) (got \(r))")
            }
            for flags: NSEvent.ModifierFlags in [[], .command, [.command, .shift]] {
                r = press(flags, sendOnReturn: on, marked: true)
                check(r.sends == 0, "IME composition never sends (on=\(on), flags=\(flags.rawValue)) (got \(r))")
            }
        }
        print(failures == 0 ? "ComposerReturn: all checks passed" : "ComposerReturn: \(failures) failed")
        exit(failures == 0 ? 0 : 1)
    }
}
