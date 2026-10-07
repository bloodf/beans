import AppKit
import XCTest
@testable import Lorca

@MainActor
final class SettingsLayoutTests: XCTestCase {
    func testLocalizedAccessoryRowsKeepLabelsSeparateFromLongChoices() {
        for appearance in [NSAppearance.Name.aqua, .darkAqua] {
            for width: CGFloat in [280, 360, 504, 760] {
                for key in ["Dictation language", "听写语言", "Automatically download updates"] {
                    let popup = SettingsPopUpButton()
                    popup.addItem(withTitle: "Automatic (English, United States) · 自动（简体中文，中国大陆）")
                    let row = AccessoryRow(key: key, accessory: popup)
                    let host = layout(row, width: width, appearance: appearance)
                    let label = row.subviews.compactMap { $0 as? NSTextField }.first!
                    XCTAssertLessThanOrEqual(label.frame.maxX + 8, popup.frame.minX + 0.5)
                    XCTAssertEqual(popup.accessibilityLabel(), key)
                    XCTAssertEqual(popup.lineBreakMode, .byTruncatingTail)
                    XCTAssertEqual(popup.toolTip, popup.titleOfSelectedItem)
                    XCTAssertTrue(popup.acceptsFirstResponder)
                    XCTAssertGreaterThanOrEqual(row.frame.height, 36)
                    XCTAssertLessThanOrEqual(popup.frame.maxX, host.bounds.maxX - 7)
                }
            }
        }
    }

    func testUpdateStatusWrapsBelowNativeActionAtNarrowWidths() {
        for width: CGFloat in [280, 360, 504, 760] {
            let row = UpdateStatusRow(key: "Version", actionTitle: "Check for Updates…")
            row.setValue("1.2.3 · Last checked today at 10:30 AM with an extended status")
            _ = layout(row, width: width, appearance: .aqua)
            let labels = row.subviews.compactMap { $0 as? NSTextField }
            let button = row.subviews.compactMap { $0 as? NSButton }.first!
            let key = labels[0].alignmentRect(forFrame: labels[0].frame)
            let action = button.alignmentRect(forFrame: button.frame)
            let status = labels[1].alignmentRect(forFrame: labels[1].frame)
            XCTAssertLessThanOrEqual(key.maxX + 10, action.minX + 0.5)
            XCTAssertLessThanOrEqual(status.width, width - 24 + 0.5)
            XCTAssertGreaterThanOrEqual(row.frame.height, 52)
            XCTAssertTrue(button.acceptsFirstResponder)
            var invoked = false
            row.onAction = { invoked = true }
            button.performClick(nil)
            XCTAssertTrue(invoked)
        }
    }

    private func layout(_ row: NSView, width: CGFloat, appearance: NSAppearance.Name) -> NSView {
        let host = NSView(frame: NSRect(x: 0, y: 0, width: width, height: 200))
        host.appearance = NSAppearance(named: appearance)
        host.addSubview(row)
        NSLayoutConstraint.activate([
            row.leadingAnchor.constraint(equalTo: host.leadingAnchor),
            row.trailingAnchor.constraint(equalTo: host.trailingAnchor),
            row.topAnchor.constraint(equalTo: host.topAnchor),
        ])
        host.layoutSubtreeIfNeeded()
        return host
    }
}
