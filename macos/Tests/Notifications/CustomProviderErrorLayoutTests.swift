import AppKit
import XCTest
@testable import Beans

@MainActor
final class CustomProviderErrorLayoutTests: XCTestCase {
    func testConnectionErrorRemainsVisibleWithoutAdvancedAndRefitsWhenCleared() throws {
        _ = NSApplication.shared
        // Construct only this controller. Do not start the store, a CLI, or a network check.
        for appearance in [NSAppearance.Name.aqua, .darkAqua] {
            let preset = try XCTUnwrap(CustomProviderPreset.local.first { $0.compatible })
            let controller = CustomProviderViewController(existing: nil, preset: preset, apiKey: "", onSave: { _ in })
            controller.loadViewIfNeeded()
            let window = NSWindow(contentViewController: controller)
            window.isReleasedWhenClosed = false
            window.appearance = NSAppearance(named: appearance)
            defer { window.contentViewController = nil; window.close() }
            XCTAssertFalse(window.isVisible)
            let advanced = try XCTUnwrap(controller.contentStack.arrangedSubviews.first {
                $0.identifier?.rawValue == "customizeModels"
            })
            XCTAssertTrue(advanced.isHidden)
            controller.show(.needsURL)
            let initialHeight = controller.preferredContentSize.height
            let shortError = "Connection refused. Check the selected Runner."
            controller.show(.failed(shortError))
            let error = try XCTUnwrap(controller.contentStack.arrangedSubviews.compactMap { $0 as? NSTextField }.first {
                $0.stringValue == shortError
            })
            XCTAssertFalse(error.isHidden)
            XCTAssertFalse(error.isHiddenOrHasHiddenAncestor)
            XCTAssertTrue(advanced.isHidden, "Showing an error must not require opening Advanced")
            XCTAssertEqual(error.maximumNumberOfLines, 0)
            XCTAssertGreaterThan(controller.preferredContentSize.height, initialHeight)
            let shortHeight = controller.preferredContentSize.height
            let longError = String(repeating: "The selected Runner cannot reach this server. Check its URL and firewall. ", count: 12)
            controller.show(.failed(longError))
            XCTAssertEqual(error.stringValue, longError)
            XCTAssertGreaterThan(controller.preferredContentSize.height, shortHeight)
            XCTAssertLessThanOrEqual(error.alignmentRect(forFrame: error.frame).width, controller.contentStack.frame.width + 1)
            XCTAssertGreaterThan(error.frame.height, 0)
            XCTAssertEqual(controller.preferredContentSize.width, 520, accuracy: 1)
            controller.show(.failed(shortError))
            XCTAssertFalse(error.isHidden)
            XCTAssertEqual(error.stringValue, shortError)
            XCTAssertEqual(controller.preferredContentSize.height, shortHeight, accuracy: 1)
            for state in [CustomProviderViewController.Listing.loading, .listed, .unlisted, .needsURL] {
                controller.show(.failed(shortError))
                controller.show(state)
                XCTAssertTrue(error.isHidden)
                XCTAssertEqual(error.stringValue, "")
                XCTAssertTrue(advanced.isHidden)
                XCTAssertEqual(controller.preferredContentSize.height, initialHeight, accuracy: 1)
            }
            XCTAssertFalse(window.isVisible)
        }
    }
}
