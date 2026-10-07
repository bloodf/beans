import AppKit
import XCTest
@testable import Beans

@MainActor
final class NativeChromeTests: XCTestCase {
    private var evidence: URL? {
        ProcessInfo.processInfo.environment["BEANS_CHROME_EVIDENCE"].map {
            URL(fileURLWithPath: $0, isDirectory: true)
        }
    }

    private func prepareAvatarGeometry() throws {
        // XCTest is not the app bundle; use the same committed JSC resource as packaging.
        guard AvatarGeometryBridge.scriptURL == nil else { return }
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().appendingPathComponent("packages/beans-blobatar/dist/blobatar.jsc.js")
        _ = try url.checkResourceIsReachable()
        AvatarGeometryBridge.scriptURL = url
    }

    func testConstructedTitlesAndPopupTextStayAligned() {
        for failure in NativeChromeRenderingChecks.run() { XCTFail(failure) }
    }

    func testImageOnlyButtonsRenderCompleteNativeSymbols() throws {
        _ = NSApplication.shared
        for appearance in [NSAppearance.Name.aqua, .darkAqua] {
            for scale: CGFloat in [1, 2] {
                for symbol in ["gearshape", "laptopcomputer", "circle.grid.2x2", "sidebar.leading", "sidebar.trailing", "terminal", "plus"] {
                    let button = HoverButton(symbol: symbol, tooltip: symbol, target: nil, action: #selector(NSResponder.cancelOperation(_:)))
                    button.frame = NSRect(x: 0, y: 0, width: 28, height: 28)
                    button.appearance = NSAppearance(named: appearance)
                    button.contentTintColor = .labelColor
                    let reference = NSImageView(frame: button.bounds)
                    reference.image = button.image
                    reference.symbolConfiguration = button.symbolConfiguration
                    reference.contentTintColor = .labelColor
                    reference.imageScaling = .scaleNone
                    reference.appearance = button.appearance
                    let name = "\(symbol)-\(appearance.rawValue)-\(Int(scale))x"
                    let actual = try render(button, scale: scale, name: name)
                    let expected = try render(reference, scale: scale, name: name + "-reference")
                    let rect = (button.cell as! NSButtonCell).imageRect(forBounds: button.bounds)
                    XCTAssertTrue(button.bounds.contains(rect), "\(name): native cell image rectangle escapes hit bounds: \(rect)")
                    let configured = button.image!.withSymbolConfiguration(button.symbolConfiguration!)!
                    XCTAssertGreaterThanOrEqual(rect.width, configured.alignmentRect.width - 0.5, name)
                    XCTAssertGreaterThanOrEqual(rect.height, configured.alignmentRect.height - 0.5, name)
                    assertSilhouette(actual, matches: expected, name: name)
                    XCTAssertEqual(button.intrinsicContentSize, NSSize(width: 28, height: 28))
                    XCTAssertNotEqual(button.focusRingType, .none)
                }
            }
        }
    }

    func testSidebarSymbolsAndFirstRowGeometry() throws {
        guard AppStore.shared.isMock else {
            throw XCTSkip("Sidebar chrome requires BEANS_MOCK=1; never load an account for rendering checks")
        }
        try prepareAvatarGeometry()
        AppStore.shared.start()
        for failure in NativeChromeRenderingChecks.runSidebar() { XCTFail(failure) }
    }

    func testReconfiguredSymbolsKeepPaletteAndButtonHitTesting() throws {
        _ = NSApplication.shared
        let button = HoverButton(symbol: "gearshape", tooltip: "Settings", target: nil, action: #selector(NSResponder.cancelOperation(_:)))
        button.frame = NSRect(x: 0, y: 0, width: 28, height: 28)
        button.image = NSImage(systemSymbolName: "laptopcomputer", accessibilityDescription: "Device")
        button.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 15, weight: .regular)
            .applying(.init(paletteColors: [.systemRed, .clear]))
        let normal = try render(button, scale: 1, name: "device-palette")
        let colors = (0..<normal.pixelsHigh).flatMap { y in
            (0..<normal.pixelsWide).compactMap { x -> NSColor? in
                let color = normal.colorAt(x: x, y: y)!.usingColorSpace(.deviceRGB)!
                return color.alphaComponent > 0.5 ? color : nil
            }
        }
        XCTAssertTrue(colors.contains { $0.redComponent > $0.greenComponent + 0.2 }, "status palette loses its red")
        XCTAssertTrue(button.hitTest(NSPoint(x: 14, y: 14)) === button, "symbol intercepts the button click")
        button.isEnabled = false
        let disabled = try render(button, scale: 1, name: "device-disabled")
        let normalAlpha = colors.map(\.alphaComponent).max()!
        let disabledAlpha = (0..<disabled.pixelsHigh).flatMap { y in
            (0..<disabled.pixelsWide).map { disabled.colorAt(x: $0, y: y)!.alphaComponent }
        }.max()!
        XCTAssertLessThan(disabledAlpha, normalAlpha, "disabled symbol stays fully emphasized")
        button.isEnabled = true
        for title in ["Back", "返回"] {
            button.label = title
            button.frame.size = button.intrinsicContentSize
            _ = try render(button, scale: 1, name: "label-\(title)")
            XCTAssertGreaterThan(button.frame.width, 28, "localized title collapses into the icon-only slot")
            XCTAssertEqual(button.accessibilityTitle(), title)
        }
        button.label = nil
        button.frame.size = button.intrinsicContentSize
        assertSilhouette(try render(button, scale: 1, name: "device-restored"), matches: normal, name: "restored icon-only symbol")
    }

    func testWindowChromeKeepsControlsInsideAccessoriesAcrossSidebarSwaps() throws {
        guard AppStore.shared.isMock else {
            throw XCTSkip("Window chrome requires BEANS_MOCK=1")
        }
        try prepareAvatarGeometry()
        AppStore.shared.start()
        _ = NSApplication.shared
        let savedSelection = Preferences.selection
        let controller = MainWindowController()
        let window = controller.window!
        window.title = "Native Chrome Window Fixture"
        window.isReleasedWhenClosed = false
        defer {
            window.orderOut(nil)
            Preferences.selection = savedSelection
        }
        for appearance in [NSAppearance.Name.aqua, .darkAqua] {
            window.appearance = NSAppearance(named: appearance)
            for width: CGFloat in [860, 1180, 1600] {
                window.setContentSize(NSSize(width: width, height: 600))
                for settings in [false, true, false] {
                    if settings { controller.root.select(.settings(.general)) }
                    else { controller.root.closeSettings() }
                    window.contentView!.layoutSubtreeIfNeeded()
                    window.displayIfNeeded()
                    let views = descendants(controller.root.view)
                    let item = controller.root.splitViewItems[0]
                    var search = views.compactMap { $0 as? SidebarSearchBar }.first
                    var footers = views.filter { $0 is SidebarFooterView }
                    if #available(macOS 26.0, *) {
                        search = search ?? item.topAlignedAccessoryViewControllers.first?.view as? SidebarSearchBar
                        footers += item.bottomAlignedAccessoryViewControllers.map(\.view).filter { bar in
                            !footers.contains { $0 === bar }
                        }
                    }
                    if let search {
                        let outlines = views.compactMap { $0 as? NSOutlineView }.filter { $0.numberOfRows > 0 }
                        if let outline = outlines.first(where: { $0.style == .sourceList }) {
                            let firstRow = outline.convert(outline.rect(ofRow: 0), to: nil)
                            let capsule = search.field.convert(search.field.bounds, to: nil)
                            XCTAssertGreaterThanOrEqual(capsule.minY, firstRow.maxY, "first row overlaps search capsule")
                            if let ring = NativeChromeRenderingChecks.nativeFocusRingBounds(search.field, scale: 2) {
                                let focusRect = search.field.convert(ring, to: nil)
                                XCTAssertGreaterThanOrEqual(focusRect.minY, firstRow.maxY, "first row overlaps native focus ring")
                            } else {
                                print("UNAVAILABLE: native focus-ring compositor; key=\(window.isKeyWindow)")
                            }
                        } else { XCTFail("first sidebar row missing; use BEANS_MOCK=1") }
                        XCTAssertNotEqual(search.field.focusRingType, .none)
                    } else { XCTFail("search missing") }
                    for footer in footers {
                        for button in descendants(footer).compactMap({ $0 as? HoverButton }) {
                            let rect = footer.convert(button.bounds, from: button)
                            XCTAssertTrue(footer.bounds.insetBy(dx: 0, dy: 4).contains(rect), "footer hit area clips")
                            XCTAssertTrue(button.visibleRect.contains(button.bounds), "footer hit area escapes visible accessory")
                        }
                    }
                    _ = try render(controller.root.view, scale: 2, name: "window-\(appearance.rawValue)-\(Int(width))-settings-\(settings)")
                }
            }
        }
    }

    private func descendants(_ view: NSView) -> [NSView] {
        view.subviews.flatMap { [$0] + descendants($0) }
    }

    private func render(_ view: NSView, scale: CGFloat, name: String) throws -> NSBitmapImageRep {
        let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(view.bounds.width * scale), pixelsHigh: Int(view.bounds.height * scale), bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        bitmap.size = view.bounds.size
        var root = view
        while let parent = root.superview { root = parent }
        let window: NSWindow?
        if root.window == nil {
            window = NSWindow(contentRect: root.bounds, styleMask: [.borderless], backing: .buffered, defer: false)
            window!.isReleasedWhenClosed = false
            window!.contentView = root
            window!.appearance = view.effectiveAppearance
            if let screen = NSScreen.screens.first(where: { $0.backingScaleFactor == scale }) {
                window!.setFrameOrigin(NSPoint(x: screen.visibleFrame.minX + 20, y: screen.visibleFrame.minY + 20))
            }
        } else { window = nil }
        root.layoutSubtreeIfNeeded()
        defer {
            window?.contentView = nil
            window?.close()
        }
        view.cacheDisplay(in: view.bounds, to: bitmap)
        if let evidence {
            try FileManager.default.createDirectory(at: evidence, withIntermediateDirectories: true)
            try bitmap.representation(using: .png, properties: [:])!.write(to: evidence.appendingPathComponent(name + ".png"))
        }
        return bitmap
    }

    private func assertSilhouette(_ actual: NSBitmapImageRep, matches expected: NSBitmapImageRep, name: String, file: StaticString = #filePath, line: UInt = #line) {
        func ink(_ bitmap: NSBitmapImageRep) -> Set<Pixel> {
            var points: Set<Pixel> = []
            for y in 0..<bitmap.pixelsHigh {
                for x in 0..<bitmap.pixelsWide where bitmap.colorAt(x: x, y: y)!.alphaComponent > 0.25 {
                    points.insert(Pixel(x: x, y: y))
                }
            }
            return points
        }
        let drawn = ink(actual)
        let wanted = ink(expected)
        XCTAssertFalse(wanted.isEmpty, "\(name): reference symbol did not render", file: file, line: line)
        guard !drawn.isEmpty, !wanted.isEmpty else {
            return XCTFail("\(name): native glyph did not render", file: file, line: line)
        }
        func bounds(_ points: Set<Pixel>) -> NSRect {
            let xs = points.map(\.x), ys = points.map(\.y)
            return NSRect(x: xs.min()!, y: ys.min()!, width: xs.max()! - xs.min()! + 1, height: ys.max()! - ys.min()! + 1)
        }
        let drawnBounds = bounds(drawn), wantedBounds = bounds(wanted)
        XCTAssertEqual(drawnBounds.width, wantedBounds.width, accuracy: 1, name, file: file, line: line)
        XCTAssertEqual(drawnBounds.height, wantedBounds.height, accuracy: 1, name, file: file, line: line)
        // AppKit aligns cells and image views to different half-pixel origins. Compare the
        // translated contour with one raster pixel of antialiasing tolerance, not alpha totals.
        func normalized(_ points: Set<Pixel>, _ rect: NSRect) -> Set<Pixel> {
            Set(points.map { Pixel(x: $0.x - Int(rect.minX), y: $0.y - Int(rect.minY)) })
        }
        let a = normalized(drawn, drawnBounds), b = normalized(wanted, wantedBounds)
        func unmatched(_ source: Set<Pixel>, _ target: Set<Pixel>) -> Int {
            source.filter { point in
                !(-1...1).contains { dy in
                    (-1...1).contains { dx in target.contains(Pixel(x: point.x + dx, y: point.y + dy)) }
                }
            }.count
        }
        XCTAssertLessThanOrEqual(unmatched(a, b) + unmatched(b, a), max(2, wanted.count / 50), "\(name): missing/distorted contour", file: file, line: line)
    }

    private struct Pixel: Hashable { let x: Int; let y: Int }
}
