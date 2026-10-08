import Foundation

/// Only schema-1 report fields can reach review, clipboard or disk. Unknown fields are discarded.
struct DiagnosticsReport: Codable {
    enum ReportError: Error { case unsupportedSchema, invalidReport }
    enum OS: String, Codable { case macos, linux, windows, ios, ipados, android, unknown }
    enum RelayReason: String, Codable { case none, not_configured, invalid_url, unreachable, update_required, http_error }
    enum ProviderKind: String, Codable { case deepseek, anthropic, opencode, opencodeGo = "opencode-go", chatgpt, grok }
    enum Health: String, Codable { case not_checked }
    enum Basis: String, Codable { case cached_setup_state }
    enum PluginState: String, Codable { case ready, needs_setup, needs_auth, connecting, error, unknown }
    enum Presence: String, Codable { case online, offline }
    struct Versions: Codable { let core: String; let relay_protocol: UInt }
    struct Device: Codable { let os: OS; let is_runner: Bool; let has_identity: Bool }
    struct Home: Codable { let exists: Bool }
    struct Port: Codable { let free: Bool }
    struct Relay: Codable {
        let configured: Bool
        let host: String?
        let reachable: Bool?
        let reason: RelayReason
        let http_status: UInt?
        let `protocol`: UInt?
    }
    struct Provider: Codable { let kind: ProviderKind; let configured: Bool }
    struct Providers: Codable { let built_in: [Provider]; let custom_configured: UInt; let health: Health }
    struct Plugin: Codable { let slot: UInt; let state: PluginState }
    struct Plugins: Codable { let basis: Basis; let entries: [Plugin] }
    struct MCP: Codable { let servers: UInt; let problems: UInt; let file_error: Bool }
    struct Runner: Codable { let is_this_device: Bool; let os: OS; let presence: Presence; let version: String? }

    let schema_version: UInt
    let versions: Versions
    let this_device: Device
    let home: Home
    let port: Port
    let relay: Relay
    let providers: Providers
    let plugins: Plugins
    let mcp_json: MCP
    let runners: [Runner]

    static func read(_ data: Data) throws -> DiagnosticsReport {
        struct Schema: Decodable { let schema_version: UInt }
        let decoder = JSONDecoder()
        guard try decoder.decode(Schema.self, from: data).schema_version == 1 else {
            throw ReportError.unsupportedSchema
        }
        let report = try decoder.decode(Self.self, from: data)
        func release(_ value: String) -> Bool {
            let parts = value.split(separator: ".", omittingEmptySubsequences: false)
            return parts.count == 3 && parts.allSatisfy { !$0.isEmpty && $0.count <= 10 && $0.utf8.allSatisfy { (48...57).contains($0) } }
        }
        guard release(report.versions.core), report.runners.allSatisfy({
            [.macos, .linux, .windows].contains($0.os) && ($0.version.map(release) ?? true)
        }) else { throw ReportError.invalidReport }
        if let host = report.relay.host {
            guard let url = URLComponents(string: "https://" + host), url.host?.isEmpty == false,
                url.user == nil, url.password == nil, url.path.isEmpty, url.query == nil,
                url.fragment == nil, url.string == "https://" + host,
                !host.unicodeScalars.contains(where: { CharacterSet.whitespacesAndNewlines.union(.controlCharacters).contains($0) })
            else { throw ReportError.invalidReport }
        }
        return report
    }

    func json() throws -> String {
        let encoder = JSONEncoder()
        var object = try JSONSerialization.jsonObject(with: encoder.encode(self)) as! [String: Any]
        var relayObject = object["relay"] as! [String: Any]
        for key in ["host", "reachable", "http_status", "protocol"] where relayObject[key] == nil {
            relayObject[key] = NSNull()
        }
        object["relay"] = relayObject
        object["runners"] = (object["runners"] as! [[String: Any]]).map { runner in
            var runner = runner
            if runner["version"] == nil { runner["version"] = NSNull() }
            return runner
        }
        return String(decoding: try JSONSerialization.data(withJSONObject: object,
            options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]), as: UTF8.self)
    }
}
