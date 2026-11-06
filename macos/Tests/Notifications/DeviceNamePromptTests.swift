import AppKit
import XCTest
@testable import Beans

@MainActor
final class DeviceNamePromptTests: XCTestCase {
    func testCancelAndRevokedOwnersDoNotProduceRenameIntent() throws {
        _ = NSApplication.shared
        let json = #"{"id":"peer","name":"Office","machine_name":"MacBook","model":"MacBook","os":"macos","os_version":"26","machine_key":"peer","is_this_device":false,"status":"offline","last_seen":0}"#
        let device = try Wire.decoder.decode(Wire.Device.self, from: Data(json.utf8)).toModel()
        XCTAssertEqual(device.name, "Office")
        XCTAssertEqual(device.machineName, "MacBook")
        let prompt = DeviceNamePrompt(device: device, owner: 7, generation: 3)
        XCTAssertTrue(prompt.alert.accessoryView === prompt.field)
        XCTAssertEqual(prompt.field.stringValue, "Office")
        prompt.field.stringValue = "Changed"
        XCTAssertNil(prompt.name(for: .alertSecondButtonReturn, owner: 7, target: "peer", generation: 3))
        XCTAssertNil(prompt.name(for: .alertFirstButtonReturn, owner: nil, target: "peer", generation: 3))
        XCTAssertNil(prompt.name(for: .alertFirstButtonReturn, owner: 8, target: "peer", generation: 3))
        XCTAssertNil(prompt.name(for: .alertFirstButtonReturn, owner: 7, target: "other", generation: 3))
        XCTAssertNil(prompt.name(for: .alertFirstButtonReturn, owner: 7, target: "peer", generation: 4))
        XCTAssertEqual(prompt.name(for: .alertFirstButtonReturn, owner: 7, target: "peer", generation: 3), "Changed")
        prompt.field.stringValue = ""
        XCTAssertEqual(prompt.name(for: .alertFirstButtonReturn, owner: 7, target: "peer", generation: 3), "")
        XCTAssertEqual(device.name, "Office", "Editing or canceling the prompt must not mutate the displayed model.")
    }
    func testTargetBindingTracksRemovalAndReplacementButNotLabels() throws {
        let json = #"{"id":"peer","name":"Office","machine_name":"MacBook","model":"MacBook","os":"macos","os_version":"26","machine_key":"peer","is_this_device":false,"status":"offline","last_seen":0}"#
        let device = try Wire.decoder.decode(Wire.Device.self, from: Data(json.utf8)).toModel()
        var ownership = DeviceNameOwnership()
        ownership.observe(.snapshotReplaced, devices: [device])
        let admitted = ownership.generation
        let original = DeviceNameTarget(device)
        var renamed = device
        renamed.name = "New label"
        renamed.machineName = "New machine label"
        XCTAssertEqual(DeviceNameTarget(renamed), original)
        ownership.observe(.rosterChanged, devices: [renamed])
        XCTAssertEqual(ownership.generation, admitted, "Label changes preserve the open prompt")
        ownership.observe(.rosterChanged, devices: [])
        ownership.observe(.rosterChanged, devices: [device])
        XCTAssertNotEqual(ownership.generation, admitted, "Remove/reappear must revoke the old prompt")
        let restored = ownership.generation
        var replaced = device
        replaced.machineKey = "replacement"
        XCTAssertNotEqual(DeviceNameTarget(replaced), original)
        ownership.observe(.rosterChanged, devices: [replaced])
        ownership.observe(.rosterChanged, devices: [device])
        XCTAssertNotEqual(ownership.generation, restored, "Key replacement and return must revoke the old prompt")
        for event in [StoreEvent.connectionChanged, .identityChanged, .snapshotReplaced] {
            let before = ownership.generation
            ownership.observe(event, devices: [device])
            XCTAssertNotEqual(ownership.generation, before)
        }
        replaced = device
        replaced.os = .linux
        XCTAssertNotEqual(DeviceNameTarget(replaced), original)
    }
}
