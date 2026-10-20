import XCTest
@testable import Beans

final class DeviceServiceStatusTests: XCTestCase {
    func testReconnectDropsCachedStatusAndRequestsAgain() throws {
        var request = DeviceServiceStatusRequest()
        request.updateOwner(1)
        let first = try XCTUnwrap(request.begin(for: "A"))
        XCTAssertTrue(request.complete(.init(installed: true, running: true), generation: first))
        XCTAssertNil(request.begin(for: "A"))
        XCTAssertNil(request.begin(for: nil))
        XCTAssertNil(request.status)
        let reconnected = try XCTUnwrap(request.begin(for: "A"))
        XCTAssertNotEqual(first, reconnected)
        XCTAssertFalse(request.complete(.init(installed: true, running: true), generation: first))
        XCTAssertTrue(request.complete(.init(installed: false, running: false), generation: reconnected))
        XCTAssertEqual(request.status?.installed, false)
    }

    func testReturningToDeviceRejectsBothOlderReplies() throws {
        var request = DeviceServiceStatusRequest()
        request.updateOwner(1)
        let firstA = try XCTUnwrap(request.begin(for: "A"))
        let b = try XCTUnwrap(request.begin(for: "B"))
        let secondA = try XCTUnwrap(request.begin(for: "A"))
        XCTAssertTrue(request.complete(.init(installed: true, running: false), generation: secondA))
        XCTAssertFalse(request.complete(.init(installed: false, running: false), generation: firstA))
        XCTAssertFalse(request.complete(.init(installed: true, running: true), generation: b))
        XCTAssertEqual(request.status?.installed, true)
        XCTAssertEqual(request.status?.running, false)
    }

    func testNewAppearanceInvalidatesInFlightReply() throws {
        var request = DeviceServiceStatusRequest()
        request.updateOwner(1)
        let old = try XCTUnwrap(request.begin(for: "A"))
        request.invalidate()
        let current = try XCTUnwrap(request.begin(for: "A"))
        XCTAssertFalse(request.complete(.init(installed: true, running: true), generation: old))
        XCTAssertTrue(request.complete(nil, generation: current))
        XCTAssertNil(request.status, "A failed request must not claim a service is absent.")
        XCTAssertNil(request.begin(for: "A"), "A failed request must not loop on every reload.")
        request.invalidate()
        XCTAssertNotNil(request.begin(for: "A"))
    }

    func testDisconnectClearsStatusAndReconnectRefetchesSameRunner() throws {
        var request = DeviceServiceStatusRequest()
        XCTAssertNil(request.begin(for: "A"), "An unadmitted store cannot inspect a service.")
        request.updateOwner(1)
        let old = try XCTUnwrap(request.begin(for: "A"))
        XCTAssertTrue(request.complete(.init(installed: true, running: true), generation: old))
        request.updateOwner(nil)
        XCTAssertNil(request.status)
        XCTAssertNil(request.begin(for: "A"))
        XCTAssertFalse(request.complete(.init(installed: false, running: false), generation: old))
        request.updateOwner(2)
        let current = try XCTUnwrap(request.begin(for: "A"))
        XCTAssertFalse(request.complete(.init(installed: true, running: true), generation: old))
        XCTAssertTrue(request.complete(.init(installed: true, running: false), generation: current))
        XCTAssertEqual(request.status?.running, false)
    }

    func testSnapshotOrAccountReplacementRejectsEarlierReplyForSameRunner() throws {
        var request = DeviceServiceStatusRequest()
        request.updateOwner(10)
        let original = try XCTUnwrap(request.begin(for: "A"))
        request.updateOwner(11)
        let snapshot = try XCTUnwrap(request.begin(for: "A"))
        XCTAssertFalse(request.complete(.init(installed: true, running: true), generation: original))
        XCTAssertTrue(request.complete(.init(installed: true, running: false), generation: snapshot))
        request.updateOwner(12)
        XCTAssertNil(request.status)
        let account = try XCTUnwrap(request.begin(for: "A"))
        XCTAssertTrue(request.complete(.init(installed: false, running: false), generation: account))
        XCTAssertFalse(request.complete(.init(installed: true, running: true), generation: snapshot))
        XCTAssertEqual(request.status?.installed, false)
    }
}
