import XCTest
@testable import Beans

final class FreshFormatTests: XCTestCase {
    private let compatible: [String: Any] = [
        "ok": true, "service": "beans-relay", "format": "beans-v2",
        "protocol": 5, "min_protocol": 5, "min_roster_protocol": 5,
        "memory_config_version": 1,
    ]

    private func health(_ fields: [String: Any]) throws -> Wire.RelayHealth {
        try Wire.RelayHealth.decode(JSONSerialization.data(withJSONObject: fields))
    }

    func testInstallationRequiresFreshFormatAndSupportedFloors() throws {
        XCTAssertTrue(try health(compatible).supports(requiredProtocol: 5))
        for (key, value) in [
            ("ok", false as Any), ("service", "another-service" as Any),
            ("format", "beans-v1" as Any), ("protocol", 4 as Any),
            ("min_protocol", 4 as Any), ("min_protocol", 6 as Any),
            ("min_roster_protocol", 4 as Any), ("min_roster_protocol", 6 as Any),
            ("memory_config_version", 2 as Any),
        ] {
            var fields = compatible
            fields[key] = value
            XCTAssertFalse(try health(fields).supports(requiredProtocol: 5), "Accepted incompatible \(key): \(value)")
        }
        XCTAssertFalse(try health(compatible).supports(requiredProtocol: 6), "An older relay cannot admit a newer release")
        var future = compatible
        future["protocol"] = 6
        future["min_roster_protocol"] = 6
        XCTAssertTrue(try health(future).supports(requiredProtocol: 6))
        XCTAssertFalse(try health(future).supports(requiredProtocol: 5), "A newer required floor blocks the current client")
    }

    func testMissingOrMalformedCompatibilityEvidenceFailsClosed() throws {
        for key in ["format", "protocol", "min_protocol", "min_roster_protocol", "memory_config_version"] {
            var fields = compatible
            fields.removeValue(forKey: key)
            XCTAssertThrowsError(try health(fields), "Missing \(key) must not select legacy defaults")
            fields[key] = NSNull()
            XCTAssertThrowsError(try health(fields), "Null \(key) must not select legacy defaults")
        }
        for key in ["protocol", "min_protocol", "min_roster_protocol", "memory_config_version"] {
            var fields = compatible
            fields[key] = "5"
            XCTAssertThrowsError(try health(fields), "A string is not integer compatibility evidence")
        }
    }

    func testLiteralDuplicateHealthEvidenceRejectsBothOrders() throws {
        let fields = [
            ("format", "\"beans-v2\"", "\"beans-v1\""),
            ("protocol", "5", "4"),
            ("min_protocol", "5", "4"),
            ("min_roster_protocol", "5", "4"),
            ("memory_config_version", "1", "2"),
        ]
        for (key, good, bad) in fields {
            let other = fields.filter { $0.0 != key }.map { "\"\($0.0)\":\($0.1)" }.joined(separator: ",")
            for (first, second) in [(good, bad), (bad, good), (good, good)] {
                let literal = "{\"ok\":true,\"service\":\"beans-relay\",\(other),\"note\":\"}\",\"\(key)\":\(first),\"\(key)\":\(second)}"
                XCTAssertThrowsError(try Wire.RelayHealth.decode(Data(literal.utf8)), "Duplicate \(key) must reject regardless of order or value")
            }
        }
        let literal = """
        {"ok":true,"service":"beans-relay","format":"beans-v2","protocol":5,"min_protocol":5,"min_roster_protocol":5,"memory_config_version":1,"extra":{"protocol":4,"memory_config_version":2},"note":"format, protocol: 4"}
        """
        XCTAssertTrue(try Wire.RelayHealth.decode(Data(literal.utf8)).supports(requiredProtocol: 5))
    }
}
