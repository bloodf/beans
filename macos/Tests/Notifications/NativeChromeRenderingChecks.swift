import AppKit
#if !BEANS_CHROME_STANDALONE
@testable import Lorca
#endif

/// Shared by XCTest and the isolated settings runner; no app store or avatar dependency.
@MainActor
enum NativeChromeRenderingChecks {
    static func run() -> [String] {
        _ = NSApplication.shared
        var failures: [String] = []
        var measurements: [String: String] = [:]
        let evidence = ProcessInfo.processInfo.environment["BEANS_CHROME_EVIDENCE"].map {
            URL(fileURLWithPath: $0, isDirectory: true)
        }
        func check(_ condition: Bool, _ message: String) {
            if !condition { failures.append(message) }
        }
        do {
            if let evidence { try FileManager.default.createDirectory(at: evidence, withIntermediateDirectories: true) }
            for appearance in [NSAppearance.Name.aqua, .darkAqua] {
                for scale: CGFloat in [1, 2] {
                    for enabled in [true, false] {
                        let button = HoverButton(symbol: "gearshape", tooltip: "Settings", target: nil, action: #selector(NSResponder.cancelOperation(_:)))
                        button.frame.size = button.intrinsicContentSize
                        button.appearance = NSAppearance(named: appearance)
                        button.isEnabled = enabled
                        let reference = NSImageView(frame: button.bounds)
                        reference.image = button.image
                        reference.symbolConfiguration = button.symbolConfiguration
                        reference.contentTintColor = button.contentTintColor
                        reference.imageScaling = .scaleNone
                        reference.alphaValue = enabled ? 1 : 0.5
                        reference.appearance = button.appearance
                        let name = "icon-gear-\(appearance.rawValue)-\(Int(scale))x-enabled-\(enabled)"
                        let actual = try render(button, scale: scale, name: name, evidence: evidence)
                        let expected = try render(reference, scale: scale, name: name + "-reference", evidence: evidence)
                        check(pixelDifference(actual, expected) == 0, "\(name): icon-only drawing changed")
                    }
                    for title in ["Back", "返回"] {
                        // Exercise the production constructor, not a later label mutation.
                        let button = HoverButton(symbol: "chevron.left", title: title, tooltip: title, target: nil, action: #selector(NSResponder.cancelOperation(_:)))
                        button.frame.size = button.intrinsicContentSize
                        button.appearance = NSAppearance(named: appearance)
                        // The word-first constructor establishes the existing attachment-only drawing path.
                        let reference = HoverButton(title: title, target: nil, action: #selector(NSResponder.cancelOperation(_:)))
                        reference.image = button.image
                        reference.symbolConfiguration = button.symbolConfiguration
                        reference.frame.size = reference.intrinsicContentSize
                        reference.appearance = button.appearance
                        let name = "constructed-\(title)-\(appearance.rawValue)-\(Int(scale))x"
                        let actual = try render(button, scale: scale, name: name, evidence: evidence)
                        let expected = try render(reference, scale: scale, name: name + "-reference", evidence: evidence)
                        let difference = pixelDifference(actual, expected)
                        measurements[name] = "differing pixels=\(difference)"
                        check(difference == 0, "\(name): extra symbol overlaps the localized word (\(difference) pixels)")
                    }
                    for choice in ["System", "Automatic (English (United States))", "系统", "自动（中文）"] {
                        let popup = SettingsPopUpButton()
                        popup.addItem(withTitle: choice)
                        let row = AccessoryRow(key: "Appearance", accessory: popup)
                        let host = NSView(frame: NSRect(x: 0, y: 0, width: 504, height: 96))
                        host.appearance = NSAppearance(named: appearance)
                        host.addSubview(row)
                        NSLayoutConstraint.activate([
                            row.leadingAnchor.constraint(equalTo: host.leadingAnchor),
                            row.trailingAnchor.constraint(equalTo: host.trailingAnchor),
                            row.centerYAnchor.constraint(equalTo: host.centerYAnchor),
                        ])
                        let name = "popup-\(choice)-\(appearance.rawValue)-\(Int(scale))x"
                        let bitmap = try render(host, scale: scale, name: name, evidence: evidence)
                        let label = row.subviews.compactMap { $0 as? NSTextField }.first!
                        let labelFrame = host.convert(label.bounds, from: label)
                        let popupFrame = host.convert(popup.bounds, from: popup)
                        let labelInk = inkBounds(bitmap, xRange: labelFrame.minX..<labelFrame.maxX, scale: scale)
                        // Exclude the chevron platter: measure the selected text, not the control frame.
                        let titleInk = inkBounds(bitmap, xRange: (popupFrame.minX + 9)..<(popupFrame.maxX - 4 - 19 - 8), scale: scale)
                        measurements[name] = "flipped=\(popup.isFlipped) row=\(row.frame) label=\(labelFrame) popup=\(popupFrame) labelInk=\(String(describing: labelInk)) titleInk=\(String(describing: titleInk))"
                        check(abs(labelFrame.midY - popupFrame.midY) <= 0.5, "\(name): accessory frame is not centered in its row")
                        if let labelInk, let titleInk {
                            check(abs(labelInk.midY - titleInk.midY) <= 2, "\(name): rendered selected title is \(abs(labelInk.midY - titleInk.midY)) pt off the row label")
                        } else { failures.append("\(name): row label or selected title failed to render") }
                    }
                }
            }
            if let evidence {
                let data = try JSONSerialization.data(withJSONObject: measurements, options: [.prettyPrinted, .sortedKeys])
                try data.write(to: evidence.appendingPathComponent("render-measurements.json"))
            }
        } catch { failures.append("native rendering fixture: \(error)") }
        return failures
    }

    private static func render(_ view: NSView, scale: CGFloat, name: String, evidence: URL?) throws -> NSBitmapImageRep {
        let window = NSWindow(contentRect: view.bounds, styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = view
        window.appearance = view.appearance
        if let screen = NSScreen.screens.first(where: { $0.backingScaleFactor == scale }) {
            window.setFrameOrigin(NSPoint(x: screen.visibleFrame.minX + 20, y: screen.visibleFrame.minY + 20))
        }
        defer { window.contentView = nil; window.close() }
        view.layoutSubtreeIfNeeded()
        let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(view.bounds.width * scale), pixelsHigh: Int(view.bounds.height * scale), bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        bitmap.size = view.bounds.size
        view.cacheDisplay(in: view.bounds, to: bitmap)
        if let evidence { try bitmap.representation(using: .png, properties: [:])!.write(to: evidence.appendingPathComponent(name + ".png")) }
        return bitmap
    }

    private static func pixelDifference(_ actual: NSBitmapImageRep, _ expected: NSBitmapImageRep) -> Int {
        guard actual.pixelsWide == expected.pixelsWide, actual.pixelsHigh == expected.pixelsHigh else { return Int.max }
        var difference = 0
        for y in 0..<actual.pixelsHigh {
            for x in 0..<actual.pixelsWide {
                if abs(actual.colorAt(x: x, y: y)!.alphaComponent - expected.colorAt(x: x, y: y)!.alphaComponent) > 0.1 { difference += 1 }
            }
        }
        return difference
    }

    private static func inkBounds(_ bitmap: NSBitmapImageRep, xRange: Range<CGFloat>, scale: CGFloat) -> NSRect? {
        var minX = bitmap.pixelsWide, minY = bitmap.pixelsHigh, maxX = -1, maxY = -1
        for y in 0..<bitmap.pixelsHigh {
            for x in max(0, Int(ceil(xRange.lowerBound * scale)))..<min(bitmap.pixelsWide, Int(floor(xRange.upperBound * scale))) {
                if bitmap.colorAt(x: x, y: y)!.alphaComponent > 0.25 {
                    minX = min(minX, x); maxX = max(maxX, x)
                    minY = min(minY, y); maxY = max(maxY, y)
                }
            }
        }
        guard maxX >= minX else { return nil }
        return NSRect(x: CGFloat(minX) / scale, y: CGFloat(minY) / scale, width: CGFloat(maxX - minX + 1) / scale, height: CGFloat(maxY - minY + 1) / scale)
    }
}
