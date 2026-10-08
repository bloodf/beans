import AppKit
import XCTest
@testable import Beans

final class RunningTaskTests: XCTestCase {
    private let start = Date(timeIntervalSince1970: 1_000)

    private func task(_ state: CommandRun.State, inGroup: Bool = false) -> RunningTask {
        RunningTask(
            id: "call", title: "Install dependencies", botName: "Scout", showsBotName: inGroup, command: "bun install",
            firstLine: "bun install", output: "", state: state, runsInForeground: false, startedAt: start)
    }

    func testTheRunningTimeCountsUpFromTheStart() {
        XCTAssertEqual(RunningTask.elapsed(since: start, now: start.addingTimeInterval(5)), "0:05")
        XCTAssertEqual(RunningTask.elapsed(since: start, now: start.addingTimeInterval(12 * 60 + 3)), "12:03")
        XCTAssertEqual(RunningTask.elapsed(since: start, now: start.addingTimeInterval(3600 + 2 * 60 + 3)), "1:02:03")
        // Another Device's clock may run behind the Runner's.
        XCTAssertEqual(RunningTask.elapsed(since: start, now: start.addingTimeInterval(-4)), "0:00")
    }

    func testTheStatusSaysWhoRunsItAndHowItStands() {
        let now = start.addingTimeInterval(65)
        XCTAssertEqual(task(.running).status(at: now), "Running · 1:05")
        XCTAssertEqual(task(.waiting, inGroup: true).status(at: now), "Scout · Waiting for input · 1:05")
        // One that ended while the list was open says how, with no running time.
        XCTAssertEqual(task(.exited).status(at: now), "Finished")
        XCTAssertEqual(task(.failed).status(at: now), "Failed")
        XCTAssertEqual(task(.stopped, inGroup: true).status(at: now), "Scout · Stopped")
    }

    func testOnlyACommandInItsTerminalIsARunningTask() {
        var run = CommandRun(command: "bun install", state: .running)
        // Before a terminal runs it: Auto-review has yet to allow it.
        XCTAssertFalse(run.takesInput)
        run.sessionID = "bash-1"
        XCTAssertTrue(run.takesInput)
        run.state = .waiting
        XCTAssertTrue(run.takesInput)
        run.state = .exited
        XCTAssertFalse(run.takesInput)
        XCTAssertTrue(run.hasEnded)
        run.state = .denied
        XCTAssertFalse(run.hasEnded)
    }

    func testRunInBackgroundIsOfferedWhileTheBotsCallWaitsOnACommandInTheForeground() {
        func row(isRunning: Bool, state: CommandRun.State = .running, sessionID: String? = "bash-1", background: Bool = false) -> Message {
            let run = CommandRun(sessionID: sessionID, command: "npm run dev", state: state, background: background)
            let tool = ToolInvocation(name: "bash", summary: "Running", detail: "", isRunning: isRunning, run: run)
            return Message(author: .bot("bot"), body: .tool(tool), createdAt: start)
        }
        XCTAssertTrue(row(isRunning: true).runsInForeground)
        XCTAssertTrue(row(isRunning: true, state: .waiting).runsInForeground)
        // Sent there, or started there and still in its first two seconds.
        XCTAssertFalse(row(isRunning: false, background: true).runsInForeground)
        XCTAssertFalse(row(isRunning: true, background: true).runsInForeground)
        XCTAssertFalse(row(isRunning: true, state: .waiting, background: true).runsInForeground)
        // Its call returned, or no terminal runs it yet.
        XCTAssertFalse(row(isRunning: false).runsInForeground)
        XCTAssertFalse(row(isRunning: true, state: .checking, sessionID: nil).runsInForeground)
        // An ended session can retain its id while the call's final update is still in flight.
        for state in [CommandRun.State.exited, .failed, .stopped] {
            XCTAssertFalse(row(isRunning: true, state: state).runsInForeground)
        }
    }

    @MainActor
    func testBlindRunningCardReservesSecureInputAndRefusesEndedOrSessionlessCards() throws {
        _ = NSApplication.shared
        var run = CommandRun(sessionID: "bash-card-check", command: "echo pick; read -r NAME </dev/tty", state: .running, handedOver: true)
        let cell = CommandCellView()
        func configure() {
            cell.configure(run: run, messageID: "card-check", botName: "Scout", avatar: nil, groupStart: true)
            cell.frame = NSRect(x: 0, y: 0, width: 480, height: CommandCellView.height(for: run, rowWidth: 480, indent: ChatMetrics.horizontalInset) + ChatMetrics.groupTopPadding)
            cell.layoutSubtreeIfNeeded()
        }
        configure()
        let secure = try XCTUnwrap(cell.subviews.compactMap { $0 as? NSSecureTextField }.first)
        let send = try XCTUnwrap(cell.subviews.compactMap { $0 as? NSButton }.first { $0.action == NSSelectorFromString("send:") })
        XCTAssertFalse(secure.isHidden)
        XCTAssertFalse(send.isHidden)
        XCTAssertGreaterThan(secure.frame.width, 0)
        XCTAssertGreaterThan(send.frame.width, 0)
        XCTAssertFalse(secure.frame.intersects(send.frame))
        XCTAssertTrue(cell.bounds.contains(secure.frame))
        XCTAssertTrue(cell.bounds.contains(send.frame))
        var dispatched = false
        cell.onSend = { _ in dispatched = true }
        for state in [CommandRun.State.exited, .failed, .stopped] {
            run.state = state
            configure()
            XCTAssertTrue(secure.isHidden)
            XCTAssertTrue(send.isHidden)
            _ = cell.perform(NSSelectorFromString("send:"), with: send)
        }
        run.state = .running
        run.sessionID = nil // Pipe-mode Runner has no terminal session.
        configure()
        XCTAssertTrue(secure.isHidden)
        XCTAssertTrue(send.isHidden)
        _ = cell.perform(NSSelectorFromString("send:"), with: send)
        XCTAssertFalse(dispatched)
    }

    @MainActor
    func testBlindInputFailureKeepsDraftWithoutReflectingErrorAndSuccessClearsIt() async throws {
        _ = NSApplication.shared
        let run = CommandRun(sessionID: "bash-card-check", command: "read -r NAME </dev/tty", state: .running, handedOver: true)
        let cell = CommandCellView()
        cell.configure(run: run, messageID: "card-check", botName: "Scout", avatar: nil, groupStart: true)
        let secure = try XCTUnwrap(cell.subviews.compactMap { $0 as? NSSecureTextField }.first)
        let send = try XCTUnwrap(cell.subviews.compactMap { $0 as? NSButton }.first { $0.action == NSSelectorFromString("send:") })
        let draft = "noncredential-card-sentinel"
        secure.stringValue = draft
        cell.onSend = { _ in throw NSError(domain: "card-check", code: 1, userInfo: [NSLocalizedDescriptionKey: draft]) }
        _ = cell.perform(NSSelectorFromString("send:"), with: send)
        for _ in 0..<100 where !send.isEnabled { await Task.yield() }
        XCTAssertTrue(send.isEnabled)
        XCTAssertEqual(secure.stringValue, draft)
        for label in cell.subviews.compactMap({ $0 as? NSTextField }) where label !== secure {
            XCTAssertFalse(label.stringValue.contains(draft))
            XCTAssertFalse(label.toolTip?.contains(draft) ?? false)
        }
        cell.onSend = { text in XCTAssertEqual(text, draft) }
        _ = cell.perform(NSSelectorFromString("send:"), with: send)
        for _ in 0..<100 where !send.isEnabled { await Task.yield() }
        XCTAssertTrue(send.isEnabled)
        XCTAssertEqual(secure.stringValue, "")
    }
}
