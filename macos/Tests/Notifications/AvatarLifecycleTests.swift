import AppKit
import XCTest
@testable import Lorca

@MainActor
final class AvatarLifecycleTests: XCTestCase {
    private func pump(_ seconds: TimeInterval) {
        let deadline = Date().addingTimeInterval(seconds)
        while Date() < deadline {
            while let event = NSApp.nextEvent(matching: .any, until: .distantPast, inMode: .default, dequeue: true) { NSApp.sendEvent(event) }
            RunLoop.main.run(until: min(deadline, Date().addingTimeInterval(0.02)))
        }
    }


    func testSidebarPresenceIsScopedToItsChat() throws {
        let store = AppStore.shared
        guard store.client.state == .disconnected else { throw XCTSkip("Does not mutate a connected account") }
        let id = store.createBot(name: "Sidebar scope regression", symbolName: "sparkles", accent: .indigo, runnerID: "test", provider: .deepseek)
        // Mock bot creation does not create a DM; own the complete graph in either mode.
        let dmID = store.dm(with: id)
        let groupID = store.createChat(kind: .group, with: [id], title: "Scope regression")
        defer {
            store.setMockWorking(id, in: groupID, false)
            store.deleteChat(groupID)
            store.deleteChat(dmID)
        }
        let dm = try XCTUnwrap(store.chat(dmID))
        store.setMockWorking(id, in: groupID, true)
        XCTAssertFalse(SidebarChatCell.Content(chat: dm, store: store).isWorking)
        XCTAssertTrue(SidebarChatCell.Content(chat: try XCTUnwrap(store.chat(groupID)), store: store).isWorking)
        store.setMockWorking(id, in: groupID, false)
        XCTAssertFalse(SidebarChatCell.Content(chat: try XCTUnwrap(store.chat(groupID)), store: store).isWorking)
    }
    private func views(_ root: NSView) -> [NSView] { [root] + root.subviews.flatMap(views) }

    func testPresenceAndPreviewStopWithMotionAndVisibility() throws {
        _ = NSApplication.shared
        AvatarGeometryBridge.scriptURL = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().appendingPathComponent("packages/beans-blobatar/dist/blobatar.jsc.js")
        let original = AvatarClock.shared.reducesMotion
        defer { AvatarClock.shared.setReducesMotion(original) }
        AvatarClock.shared.setReducesMotion(false)
        let controller = BotLookViewController(botID: "lifecycle-only")
        let window = NSWindow(contentRect: NSRect(x: 200, y: 200, width: 520, height: 760), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentViewController = controller
        window.orderFrontRegardless()
        pump(0.5)
        defer { window.orderOut(nil); controller.viewWillDisappear(); window.close() }
        guard window.occlusionState.contains(.visible) else { throw XCTSkip("Window server did not expose the test window") }
        controller.viewDidAppear()
        let cycle = try XCTUnwrap(views(controller.view).compactMap { $0 as? NSButton }.first { $0.action == NSSelectorFromString("toggleCycle") })
        let presence = PresenceLayer()
        presence.sync(isWorking: true, in: CGRect(x: 0, y: 0, width: 40, height: 40), on: controller.view.layer, flipped: false, view: controller.view)
        XCTAssertNotNil(presence.animation(forKey: "breathe"))
        cycle.performClick(nil)
        XCTAssertEqual(cycle.state, .on)
        AvatarClock.shared.setReducesMotion(true)
        XCTAssertNil(presence.animation(forKey: "breathe"))
        XCTAssertEqual(cycle.state, .off)
        XCTAssertFalse(cycle.isEnabled)
        XCTAssertEqual(AvatarClock.shared.activeCount, 0)
        AvatarClock.shared.setReducesMotion(false)
        XCTAssertTrue(cycle.isEnabled)
        cycle.performClick(nil)
        XCTAssertEqual(cycle.state, .on)
        window.orderOut(nil)
        pump(0.2)
        presence.refreshMotion()
        XCTAssertNil(presence.animation(forKey: "breathe"))
        XCTAssertEqual(cycle.state, .off)
        XCTAssertEqual(AvatarClock.shared.activeCount, 0)
    }

    func testFailedCombinedSaveRetainsEditorAndStoreDraft() async throws {
        let store = AppStore.shared
        guard !store.isMock, store.client.state == .disconnected else { throw XCTSkip("Requires a disconnected, non-mock store; does not start a CLI") }
        let id = store.createBot(name: "Look failure regression", symbolName: "sparkles", accent: .indigo, runnerID: "test", provider: .deepseek)
        defer { if let chat = store.chats.first(where: { $0.isDM && $0.botIDs == [id] }) { store.deleteChat(chat.id) } }
        let before = try XCTUnwrap(store.bot(id))
        var look = BotLook()
        look.base.shape = .sun
        do {
            try await store.saveBotLook(id, look: .replace(look), photo: .remove)
            XCTFail("Disconnected save must fail")
        } catch {
            XCTAssertEqual(store.bot(id), before, "No optimistic half-save")
        }
        AvatarGeometryBridge.scriptURL = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().appendingPathComponent("packages/beans-blobatar/dist/blobatar.jsc.js")
        let editor = BotLookViewController(botID: id)
        let all = views(editor.view)
        let shuffle = try XCTUnwrap(all.compactMap { $0 as? NSButton }.first { $0.action == NSSelectorFromString("shuffle") })
        shuffle.performClick(nil)
        let preview = try XCTUnwrap(all.compactMap { $0 as? AvatarView }.first { $0.diameter == 96 })
        let draft = preview.content
        editor.confirmTapped()
        await Task.yield()
        pump(0.05)
        XCTAssertEqual(preview.content, draft, "Failure retains generated draft")
        XCTAssertTrue(editor.confirmButton.isEnabled, "Save can be retried")
        XCTAssertEqual(store.bot(id), before)
        XCTAssertTrue(all.compactMap { $0 as? NSTextField }.contains { $0.stringValue.contains("not saved") })
    }

    func testPendingSaveBlocksCancelAndEscapeUntilRejection() async throws {
        let store = AppStore.shared
        guard !store.isMock, store.client.state == .disconnected else {
            throw XCTSkip("Requires a disconnected, non-mock store; does not start a CLI")
        }
        _ = NSApplication.shared
        AvatarGeometryBridge.scriptURL = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().appendingPathComponent("packages/beans-blobatar/dist/blobatar.jsc.js")
        for escape in [false, true] {
            let id = store.createBot(name: "Pending Look regression", symbolName: "sparkles", accent: .indigo, runnerID: "test", provider: .deepseek)
            defer {
                if let chat = store.chats.first(where: { $0.isDM && $0.botIDs == [id] }) {
                    store.deleteChat(chat.id)
                }
            }
            let before = try XCTUnwrap(store.bot(id))
            let started = expectation(description: "Save is awaiting its response")
            var pending: CheckedContinuation<Void, Error>?
            let rejection = NSError(domain: "LookSaveRegression", code: 1,
                userInfo: [NSLocalizedDescriptionKey: "Delayed save rejected"])
            let editor = BotLookViewController(botID: id) { _, _, _ in
                try await withCheckedThrowingContinuation { continuation in
                    pending = continuation
                    started.fulfill()
                }
            }
            let presenter = NSViewController()
            presenter.view = NSView(frame: NSRect(x: 0, y: 0, width: 600, height: 800))
            let window = NSWindow(contentRect: presenter.view.frame, styleMask: [.titled], backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentViewController = presenter
            window.orderFrontRegardless()
            presenter.presentAsSheet(editor)
            pump(0.1)
            defer {
                pending?.resume(throwing: rejection)
                editor.dismiss(nil)
                window.orderOut(nil)
                window.close()
            }
            let all = views(editor.view)
            let shape = try XCTUnwrap(all.compactMap { $0 as? NSPopUpButton }
                .first { $0.itemTitles.contains(L("Triangle")) })
            shape.selectItem(withTitle: L("Sun"))
            XCTAssertTrue(NSApp.sendAction(try XCTUnwrap(shape.action), to: shape.target, from: shape))
            let preview = try XCTUnwrap(all.compactMap { $0 as? AvatarView }.first { $0.diameter == 96 })
            let draft = preview.content
            let cancel = try XCTUnwrap(all.compactMap { $0 as? NSButton }
                .first { $0.action == #selector(SheetViewController.dismissSheet) })
            XCTAssertTrue(presenter.presentedViewControllers?.contains { $0 === editor } == true)
            editor.confirmTapped()
            await fulfillment(of: [started], timeout: 2)
            XCTAssertFalse(editor.confirmButton.isEnabled)
            XCTAssertFalse(cancel.isEnabled, "Cancel is disabled while Save awaits a response")
            if escape {
                editor.cancelOperation(nil)
            } else {
                // Also exercise the action itself: stale queued actions cannot dismiss.
                XCTAssertTrue(NSApp.sendAction(try XCTUnwrap(cancel.action), to: cancel.target, from: cancel))
            }
            pump(0.1)
            XCTAssertTrue(presenter.presentedViewControllers?.contains { $0 === editor } == true,
                "\(escape ? "Escape" : "Cancel") must not dismiss a pending save")
            XCTAssertNotNil(window.attachedSheet)
            let continuation = try XCTUnwrap(pending)
            pending = nil
            continuation.resume(throwing: rejection)
            let deadline = Date().addingTimeInterval(2)
            while !editor.confirmButton.isEnabled && Date() < deadline {
                await Task.yield()
                pump(0.01)
            }
            XCTAssertTrue(presenter.presentedViewControllers?.contains { $0 === editor } == true)
            XCTAssertEqual(preview.content, draft, "Rejection retains the edited look")
            XCTAssertEqual(store.bot(id), before, "Rejection does not change the saved bot")
            XCTAssertTrue(editor.confirmButton.isEnabled, "Save can be retried")
            XCTAssertTrue(cancel.isEnabled, "Cancel is restored after rejection")
            XCTAssertTrue(all.compactMap { $0 as? NSTextField }
                .contains { $0.stringValue.contains("Delayed save rejected") })
            editor.cancelOperation(nil)
            pump(0.1)
            XCTAssertFalse(presenter.presentedViewControllers?.contains { $0 === editor } == true,
                "Escape dismisses normally after the save has failed")
        }
    }
}
