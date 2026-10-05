import XCTest
@testable import Lorca

final class MCPServerTests: XCTestCase {
    func testRunnerListingDecodesStatusAndKeepsConfigTypes() throws {
        let json = """
            {"path":"/home/runner/.lorca/mcp.json","error":null,"servers":[{"name":"lab","id":"mcp:lab","transport":"stdio","enabled":false,"config":{"command":"node","args":["server.js"],"env":{"TOKEN":"private"},"disabled":true},"problem":null,"status":{"id":"mcp:lab","name":"lab","description":"","version":"","icon":"","state":"ready","detail":"Ready"},"signs_in":false,"signed_in":false,"tool_count":1}]}
            """
        let listing = try JSONDecoder().decode(MCPServerList.self, from: Data(json.utf8))
        let server = try XCTUnwrap(listing.servers.first)
        XCTAssertFalse(server.enabled)
        XCTAssertEqual(server.status?.detail, "Ready")
        let object = server.config.requestValue
        let encoded = try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
        let result = try XCTUnwrap(String(data: encoded, encoding: .utf8))
        XCTAssertTrue(result.contains("\"disabled\":true"))
        XCTAssertTrue(result.contains("\"args\":[\"server.js\"]"))
        XCTAssertTrue(result.contains("\"TOKEN\":\"private\""))
    }

    func testMalformedEntryDoesNotBreakOtherServers() throws {
        let json = """
            {"path":"mcp.json","error":null,"servers":[{"name":"bad","id":"mcp:bad","transport":"unknown","enabled":true,"config":null,"problem":"invalid config","status":null,"signs_in":false,"signed_in":false,"tool_count":0}]}
            """
        let listing = try JSONDecoder().decode(MCPServerList.self, from: Data(json.utf8))
        XCTAssertEqual(listing.servers.first?.problem, "invalid config")
        if case .null = listing.servers[0].config {} else { XCTFail("must preserve invalid JSON config") }
    }

    func testPastedMultiServerParseReplyPlansEachServerWithoutOverwriting() throws {
        let json = """
            {"servers":[{"name":"memory","config":{"command":"node","args":["memory.js"]},"problem":null},
                        {"name":"search","config":{"type":"http","url":"https://example.test/mcp"},"problem":null}]}
            """
        let parsed = try JSONDecoder().decode(MCPParsedServers.self, from: Data(json.utf8))
        let saves = try MCPServerViewController.plannedSaves(parsed, name: "", previousName: nil, existingNames: [])
        XCTAssertEqual(saves.map(\.0), ["memory", "search"])
        XCTAssertEqual(saves[0].1["command"], .string("node"))
        XCTAssertThrowsError(try MCPServerViewController.plannedSaves(parsed, name: "", previousName: nil, existingNames: ["search"]))
        XCTAssertThrowsError(try MCPServerViewController.plannedSaves(parsed, name: "", previousName: "memory", existingNames: ["memory"]))
    }

    func testInvalidSecondServerPreventsWholeBatch() throws {
        let json = """
            {"servers":[{"name":"ok","config":{"command":"node"},"problem":null},
                        {"name":"broken","config":null,"problem":"missing command"}]}
            """
        let parsed = try JSONDecoder().decode(MCPParsedServers.self, from: Data(json.utf8))
        XCTAssertThrowsError(try MCPServerViewController.plannedSaves(parsed, name: "", previousName: nil, existingNames: []))
    }

    func testLiveControlsRebaseUnsavedConfigWithoutExposingKey() {
        let old: MCPJSON = .object(["disabled": .bool(false), "env": .object(["API_KEY": .string("••••••••")]), "description": .string("old")])
        let draft: MCPJSON = .object(["disabled": .bool(false), "env": .object(["API_KEY": .string("••••••••")]), "description": .string("user edit")])
        let live: MCPJSON = .object(["disabled": .bool(true), "env": .object(["API_KEY": .string("••••••••")]), "description": .string("old"), "toolExposure": .object(["read": .string("hidden")])])
        XCTAssertEqual(MCPServerViewController.rebaseDraft(draft, from: old, to: live),
            .object(["disabled": .bool(true), "env": .object(["API_KEY": .string("••••••••")]), "description": .string("user edit"), "toolExposure": .object(["read": .string("hidden")])]))
    }
}
