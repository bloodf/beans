// Run with macos/Tests/run-settings-layout.sh; exercises production AppKit rows without launching the app.
import AppKit

// Controls.swift's unrelated clipboard and presence controls need only these domain names.
func L(_ text: String) -> String { text }
enum Device { enum Status { case online, pairing, offline } }

@main
struct SettingsLayoutChecks {
    @MainActor
    static func main() {
        _ = NSApplication.shared
        var failures: [String] = []
        func check(_ condition: Bool, _ message: String) {
            if !condition { failures.append(message) }
        }
        for appearance in [NSAppearance.Name.aqua, .darkAqua] {
            for width: CGFloat in [280, 360, 504, 760] {
                for (key, choice) in [
                    ("Appearance", "System"),
                    ("Dictation language", "Automatic (English, United States, extended language name)"),
                    ("Automatically download updates", "On"),
                    ("听写语言", "自动（简体中文，中国大陆，扩展语言名称）"),
                    ("自动下载更新", "开启"),
                ] {
                    let popup = SettingsPopUpButton()
                    popup.addItem(withTitle: choice)
                    let row = AccessoryRow(key: key, accessory: popup)
                    let host = NSView(frame: NSRect(x: 0, y: 0, width: width, height: 200))
                    host.appearance = NSAppearance(named: appearance)
                    host.addSubview(row)
                    NSLayoutConstraint.activate([
                        row.leadingAnchor.constraint(equalTo: host.leadingAnchor),
                        row.trailingAnchor.constraint(equalTo: host.trailingAnchor),
                        row.topAnchor.constraint(equalTo: host.topAnchor),
                    ])
                    host.layoutSubtreeIfNeeded()
                    let label = row.subviews.compactMap { $0 as? NSTextField }.first!
                    let context = "\(appearance.rawValue) \(width) \(key)"
                    check(label.frame.maxX + 8 <= popup.frame.minX + 0.5, "overlap: \(context)")
                    check(label.frame.width >= min(60, label.intrinsicContentSize.width), "label collapsed: \(context)")
                    check(popup.lineBreakMode == .byTruncatingTail, "popup title does not truncate: \(context)")
                    check(row.frame.height >= 36, "row rhythm: \(context)")
                    check(popup.frame.maxX <= row.bounds.maxX - 7, "accessory overflow: \(context)")
                    check(popup.accessibilityLabel() == key, "missing control label: \(context)")
                    check(popup.acceptsFirstResponder, "popup not keyboard reachable: \(context)")
                    check(popup.titleOfSelectedItem == choice, "selected title changed: \(context)")
                    check(popup.toolTip == choice, "full choice tooltip missing: \(context)")
                }
            }
        }
        for width: CGFloat in [280, 360, 504, 760] {
            let toggle = NSSwitch()
            toggle.controlSize = .small
            let keyText = "Automatically download updates with an extended localized setting label"
            let row = AccessoryRow(key: keyText, accessory: toggle)
            let host = NSView(frame: NSRect(x: 0, y: 0, width: width, height: 200))
            host.addSubview(row)
            NSLayoutConstraint.activate([
                row.leadingAnchor.constraint(equalTo: host.leadingAnchor),
                row.trailingAnchor.constraint(equalTo: host.trailingAnchor),
                row.topAnchor.constraint(equalTo: host.topAnchor),
            ])
            host.layoutSubtreeIfNeeded()
            let label = row.subviews.compactMap { $0 as? NSTextField }.first!
            check(label.frame.maxX + 8 <= toggle.frame.minX + 0.5, "switch overlap: \(width)")
            check(label.frame.minY >= 6 && label.frame.maxY <= row.bounds.maxY - 6, "wrapping label escapes row: \(width)")
            check(toggle.accessibilityLabel() == keyText, "switch label missing")
            check(toggle.acceptsFirstResponder, "switch not keyboard reachable")
            if width == 280 { check(row.frame.height > 36, "long switch label must grow row") }
        }
        for width: CGFloat in [280, 360, 504, 760] {
            for (key, action, status) in [
                ("Version", "Check for Updates…", "1.2.3 · Last checked today at 10:30 AM with an extended status"),
                ("版本", "检查更新…", "1.2.3 · 今天上午十点三十分检查，扩展状态说明"),
            ] {
                let row = UpdateStatusRow(key: key, actionTitle: action)
                row.setValue(status)
                let host = NSView(frame: NSRect(x: 0, y: 0, width: width, height: 200))
                host.addSubview(row)
                NSLayoutConstraint.activate([
                    row.leadingAnchor.constraint(equalTo: host.leadingAnchor),
                    row.trailingAnchor.constraint(equalTo: host.trailingAnchor),
                    row.topAnchor.constraint(equalTo: host.topAnchor),
                ])
                host.layoutSubtreeIfNeeded()
                let labels = row.subviews.compactMap { $0 as? NSTextField }
                let button = row.subviews.compactMap { $0 as? NSButton }.first!
                let keyRect = labels[0].alignmentRect(forFrame: labels[0].frame)
                let buttonRect = button.alignmentRect(forFrame: button.frame)
                let statusRect = labels[1].alignmentRect(forFrame: labels[1].frame)
                check(keyRect.maxX + 10 <= buttonRect.minX + 0.5, "updates action overlap: \(width)")
                check(statusRect.width <= width - 24 + 0.5, "updates status overflow: \(width)")
                check(row.frame.height >= 52, "updates status needs its own line: \(width)")
                check(button.acceptsFirstResponder, "updates action not keyboard reachable")
                check(labels[1].stringValue == status, "updates status changed")
            }
        }
        guard failures.isEmpty else {
            failures.forEach { print("FAIL: \($0)") }
            exit(1)
        }
        print("PASS: settings rows, English/Chinese, 280/360/504/760 pt, light/dark, control labels and keyboard eligibility")
    }
}
