import AppKit
#if !BEANS_CHROME_STANDALONE
@testable import Beans
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

    #if !BEANS_CHROME_STANDALONE || BEANS_SIDEBAR_STANDALONE
    static func runSidebar() -> [String] {
        _ = NSApplication.shared
        var failures: [String] = []
        var measurements: [String: String] = [:]
        var unavailableFocusRings = 0
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
                    for width: CGFloat in [212, 260, 320] {
                        for pane in SettingsPane.allCases {
                            let row = SidebarPaneCell()
                            row.frame = NSRect(x: 0, y: 0, width: width, height: 32)
                            row.appearance = NSAppearance(named: appearance)
                            row.configure(pane: pane)
                            let rowWindow = NSWindow(contentRect: row.frame, styleMask: [.borderless], backing: .buffered, defer: false)
                            rowWindow.isReleasedWhenClosed = false
                            rowWindow.contentView = row
                            rowWindow.appearance = row.appearance
                            if let screen = NSScreen.screens.first(where: { $0.backingScaleFactor == scale }) {
                                rowWindow.setFrameOrigin(NSPoint(x: screen.visibleFrame.minX + 20, y: screen.visibleFrame.minY + 20))
                            }
                            row.layoutSubtreeIfNeeded()
                            defer { rowWindow.contentView = nil; rowWindow.close() }
                            let icon = row.subviews.compactMap { $0 as? NSImageView }.first!
                            // Independent roomy native reference: never inherit the actual glyph box or configuration.
                            let reference = NSImageView(frame: NSRect(x: 0, y: 0, width: 64, height: 64))
                            reference.image = NSImage(systemSymbolName: pane.symbolName, accessibilityDescription: nil)
                            reference.symbolConfiguration = .init(pointSize: 15, weight: .regular)
                            reference.imageScaling = .scaleNone
                            reference.contentTintColor = .secondaryLabelColor
                            reference.appearance = row.appearance
                            let name = "pane-\(pane.rawValue)-\(appearance.rawValue)-\(Int(width))-\(Int(scale))x"
                            check(row.bounds.contains(icon.frame), "\(name): complete glyph box escapes the native row")
                            check(icon.frame.maxX <= row.textField!.frame.minX, "\(name): glyph overlaps label column")
                            let capture = width == 212 && (pane == .bots || pane == .providers) ? evidence : nil
                            let drawn = try renderAttached(icon, scale: scale, name: name, evidence: capture)
                            let wanted = try render(reference, scale: scale, name: name + "-reference", evidence: capture)
                            let mismatch = contourDifference(drawn, wanted)
                            check(mismatch == nil, "\(name): \(mismatch ?? "")")
                            if width == 212 && scale == 2 && appearance == .aqua {
                                let configured = reference.image!.withSymbolConfiguration(reference.symbolConfiguration!)!
                                let measurement = "symbolSize=\(configured.size) alignment=\(configured.alignmentRect) scaling=\(icon.imageScaling.rawValue) frame=\(icon.frame) ink=\(String(describing: inkBounds(drawn, xRange: 0..<icon.bounds.width, scale: scale)))"
                                measurements[pane.rawValue] = measurement
                                print("MEASURE \(pane.rawValue): \(measurement)")
                            }
                        }
                    }
                    let chats = SidebarViewController()
                    let settings = SettingsSidebarViewController()
                    let split = NSSplitViewController()
                    let sidebarHost = NSViewController()
                    sidebarHost.view = NSView()
                    let item = NSSplitViewItem(sidebarWithViewController: sidebarHost)
                    item.minimumThickness = 232
                    item.maximumThickness = 340
                    split.addSplitViewItem(item)
                    split.addSplitViewItem(NSSplitViewItem(viewController: NSViewController()))
                    let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1180, height: 600),
                        styleMask: [.titled, .resizable, .fullSizeContentView], backing: .buffered, defer: false)
                    window.isReleasedWhenClosed = false
                    window.title = "Sidebar Chrome Fixture"
                    window.toolbar = NSToolbar(identifier: "SidebarChromeFixture")
                    window.toolbarStyle = .unified
                    window.appearance = NSAppearance(named: appearance)
                    window.contentViewController = split
                    if let screen = NSScreen.screens.first(where: { $0.backingScaleFactor == scale }) {
                        window.setFrameOrigin(NSPoint(x: screen.visibleFrame.minX + 20, y: screen.visibleFrame.minY + 20))
                    }
                    defer { window.contentViewController = nil; window.close() }
                    for windowWidth: CGFloat in [860, 1180, 1600] {
                        window.setContentSize(NSSize(width: windowWidth, height: 600))
                        for sidebarWidth: CGFloat in [232, 260, 340] {
                            split.splitView.setPosition(sidebarWidth, ofDividerAt: 0)
                            for (swap, isSettings) in [false, true, false].enumerated() {
                                let controller: NSViewController = isSettings ? settings : chats
                                sidebarHost.children.forEach { $0.removeFromParent(); $0.view.removeFromSuperview() }
                                sidebarHost.addChild(controller)
                                sidebarHost.view.addSubview(controller.view)
                                controller.view.pin(to: sidebarHost.view)
                                let bar = (isSettings ? settings.header : chats.searchBar) as! SidebarSearchBar
                                let footer = isSettings ? settings.footer : chats.footer
                                if #available(macOS 26.0, *) {
                                    let top = NSSplitViewItemAccessoryViewController()
                                    top.view = bar
                                    top.automaticallyAppliesContentInsets = true
                                    let bottom = NSSplitViewItemAccessoryViewController()
                                    bottom.view = footer
                                    bottom.automaticallyAppliesContentInsets = true
                                    item.topAlignedAccessoryViewControllers = [top]
                                    item.bottomAlignedAccessoryViewControllers = [bottom]
                                }
                                window.contentView!.layoutSubtreeIfNeeded()
                                window.displayIfNeeded()
                                let name = "sidebar-\(appearance.rawValue)-\(Int(scale))x-\(Int(windowWidth))-\(Int(sidebarWidth))-swap-\(swap)-settings-\(isSettings)"
                                let views = descendants(controller.view)
                                guard let outline = views.compactMap({ $0 as? NSOutlineView }).first,
                                    let scroll = views.compactMap({ $0 as? NSScrollView }).first,
                                    outline.numberOfRows > 0 else {
                                    failures.append("\(name): first outline row missing")
                                    continue
                                }
                                let fieldRect = bar.field.convert(bar.field.bounds, to: nil)
                                let stripRect = bar.convert(bar.bounds, to: nil)
                                let firstRow = outline.convert(outline.rect(ofRow: 0), to: nil)
                                let ring = nativeFocusRingBounds(bar.field, scale: scale).map {
                                    bar.field.convert($0, to: nil)
                                }
                                let measurement = "field=\(fieldRect) intrinsic=\(bar.field.intrinsicContentSize) strip=\(stripRect) firstRow=\(firstRow) nativeRing=\(String(describing: ring)) gap=\(fieldRect.minY - firstRow.maxY) hostSafe=\(sidebarHost.view.safeAreaInsets) safe=\(controller.view.safeAreaInsets) content=\(scroll.contentInsets) key=\(window.isKeyWindow) backing=\(window.backingScaleFactor)"
                                measurements[name] = measurement
                                if windowWidth == 1180 && sidebarWidth == 260 && swap < 2 { print("MEASURE \(name): \(measurement)") }
                                check(fieldRect.minY >= firstRow.maxY, "\(name): first row overlaps search capsule")
                                if let ring {
                                    check(ring.minY >= firstRow.maxY, "\(name): first row overlaps native focus ring")
                                    check(stripRect.contains(ring), "\(name): native focus ring escapes search strip")
                                } else { unavailableFocusRings += 1 }
                                if SidebarChrome.floats {
                                    check(abs(scroll.contentInsets.top - sidebarHost.view.safeAreaInsets.top) <= 0.5,
                                        "\(name): list adds \(scroll.contentInsets.top - sidebarHost.view.safeAreaInsets.top) pt beyond AppKit accessory inset")
                                }
                                check(bar.field.focusRingType != .none, "\(name): native search focus ring removed")
                                let reference = NSSearchField()
                                reference.controlSize = .regular
                                check(abs(fieldRect.height - reference.intrinsicContentSize.height) <= 0.5,
                                    "\(name): sidebar search is \(fieldRect.height) pt; regular native search is \(reference.intrinsicContentSize.height) pt")
                                for button in descendants(footer).compactMap({ $0 as? HoverButton }) {
                                    check(button.visibleRect.contains(button.bounds), "\(name): footer hit area clips")
                                }
                                if isSettings && appearance == .aqua && scale == 2 && windowWidth == 1180 && sidebarWidth == 260 {
                                    check(window.makeFirstResponder(bar.field), "\(name): native search cannot take keyboard focus")
                                    if let editor = bar.field.currentEditor() as? NSTextView {
                                        editor.insertText("zz-no-native-sidebar-result", replacementRange: NSRange(location: NSNotFound, length: 0))
                                        check(outline.numberOfRows == 0, "\(name): typed search did not filter the outline")
                                        let cell = bar.field.cell as! NSSearchFieldCell
                                        cell.cancelButtonCell?.performClick(bar.field)
                                        check(bar.field.stringValue.isEmpty && outline.numberOfRows == SettingsPane.allCases.count,
                                            "\(name): native clear did not restore panes")
                                        let down = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0,
                                            windowNumber: window.windowNumber, context: nil, characters: "\u{f701}",
                                            charactersIgnoringModifiers: "\u{f701}", isARepeat: false, keyCode: 125)!
                                        editor.interpretKeyEvents([down])
                                        check(window.firstResponder === outline, "\(name): Down arrow did not move focus to outline")
                                        outline.selectRowIndexes(IndexSet(integer: 1), byExtendingSelection: false)
                                        check(outline.selectedRow == 1, "\(name): native pane selection failed")
                                    } else { failures.append("\(name): native search field editor missing") }
                                }
                                let capture = windowWidth == 1180 && sidebarWidth == 260 && swap < 2 ? evidence : nil
                                if let capture {
                                    _ = try renderAttached(bar, scale: scale, name: name + "-search", evidence: capture)
                                }
                            }
                        }
                    }
                }
            }
            if let evidence {
                try JSONSerialization.data(withJSONObject: measurements, options: [.prettyPrinted, .sortedKeys])
                    .write(to: evidence.appendingPathComponent("sidebar-measurements.json"))
            }
            print("UNAVAILABLE: \(unavailableFocusRings) native focus-ring probes; inactive macOS compositor geometry is not acceptance")
        } catch { failures.append("sidebar rendering fixture: \(error)") }
        return failures
    }

    private static func descendants(_ view: NSView) -> [NSView] {
        view.subviews.flatMap { [$0] + descendants($0) }
    }

    // Draw AppKit's own cell mask with its native focus-ring style into a roomy bitmap.
    // This measures geometry without pretending that an inactive fixture is a key window.
    static func nativeFocusRingBounds(_ field: NSSearchField, scale: CGFloat) -> NSRect? {
        let margin: CGFloat = 16
        let size = NSSize(width: field.bounds.width + 2 * margin, height: field.bounds.height + 2 * margin)
        let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(size.width * scale), pixelsHigh: Int(size.height * scale), bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        bitmap.size = size
        NSGraphicsContext.saveGraphicsState()
        defer { NSGraphicsContext.restoreGraphicsState() }
        NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
        let transform = NSAffineTransform()
        transform.scale(by: scale)
        transform.translateX(by: margin, yBy: margin)
        transform.concat()
        field.effectiveAppearance.performAsCurrentDrawingAppearance {
            NSFocusRingPlacement.only.set()
            field.cell!.drawFocusRingMask(withFrame: field.bounds, in: field)
        }
        guard let ink = inkBounds(bitmap, xRange: 0..<size.width, scale: scale) else { return nil }
        return NSRect(x: ink.minX - margin, y: size.height - ink.maxY - margin, width: ink.width, height: ink.height)
    }

    private static func renderAttached(_ view: NSView, scale: CGFloat, name: String, evidence: URL?) throws -> NSBitmapImageRep {
        let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(view.bounds.width * scale), pixelsHigh: Int(view.bounds.height * scale), bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        bitmap.size = view.bounds.size
        view.cacheDisplay(in: view.bounds, to: bitmap)
        if let evidence { try bitmap.representation(using: .png, properties: [:])!.write(to: evidence.appendingPathComponent(name + ".png")) }
        return bitmap
    }

    private struct Pixel: Hashable { let x: Int; let y: Int }

    private static func contourDifference(_ actual: NSBitmapImageRep, _ expected: NSBitmapImageRep) -> String? {
        func ink(_ bitmap: NSBitmapImageRep) -> Set<Pixel> {
            var result: Set<Pixel> = []
            for y in 0..<bitmap.pixelsHigh {
                for x in 0..<bitmap.pixelsWide where bitmap.colorAt(x: x, y: y)!.alphaComponent > 0.25 {
                    result.insert(Pixel(x: x, y: y))
                }
            }
            return result
        }
        let a = ink(actual), b = ink(expected)
        guard !a.isEmpty, !b.isEmpty else { return "native glyph/reference did not render" }
        func extent(_ pixels: Set<Pixel>) -> (Int, Int, Int, Int) {
            let xs = pixels.map(\.x), ys = pixels.map(\.y)
            return (xs.min()!, ys.min()!, xs.max()! - xs.min()! + 1, ys.max()! - ys.min()! + 1)
        }
        let ab = extent(a), bb = extent(b)
        guard abs(ab.2 - bb.2) <= 1, abs(ab.3 - bb.3) <= 1 else {
            return "ink \(ab.2)×\(ab.3) px; independent 15-pt reference \(bb.2)×\(bb.3) px"
        }
        let translatedA = Set(a.map { Pixel(x: $0.x - ab.0, y: $0.y - ab.1) })
        let translatedB = Set(b.map { Pixel(x: $0.x - bb.0, y: $0.y - bb.1) })
        func unmatched(_ source: Set<Pixel>, _ target: Set<Pixel>) -> Int {
            source.filter { point in
                !(-1...1).contains { dy in
                    (-1...1).contains { dx in target.contains(Pixel(x: point.x + dx, y: point.y + dy)) }
                }
            }.count
        }
        let missing = unmatched(translatedA, translatedB) + unmatched(translatedB, translatedA)
        return missing <= max(2, b.count / 50) ? nil : "missing/distorted contour (\(missing) pixels)"
    }
    #endif

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
