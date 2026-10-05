import Foundation

/// JSON config stays on the Runner; never put it in roster state or logs.
indirect enum MCPJSON: Codable, Equatable {
    case object([String: MCPJSON]), array([MCPJSON]), string(String), number(Double), bool(Bool), null

    init(from decoder: Decoder) throws {
        let value = try decoder.singleValueContainer()
        if value.decodeNil() { self = .null }
        else if let bool = try? value.decode(Bool.self) { self = .bool(bool) }
        else if let number = try? value.decode(Double.self) { self = .number(number) }
        else if let string = try? value.decode(String.self) { self = .string(string) }
        else if let object = try? value.decode([String: MCPJSON].self) { self = .object(object) }
        else { self = .array(try value.decode([MCPJSON].self)) }
    }

    func encode(to encoder: Encoder) throws {
        var value = encoder.singleValueContainer()
        switch self {
        case let .object(object): try value.encode(object)
        case let .array(array): try value.encode(array)
        case let .string(string): try value.encode(string)
        case let .number(number): try value.encode(number)
        case let .bool(bool): try value.encode(bool)
        case .null: try value.encodeNil()
        }
    }

    var requestValue: Any {
        switch self {
        case let .object(object): return object.mapValues(\.requestValue)
        case let .array(array): return array.map(\.requestValue)
        case let .string(string): return string
        case let .number(number): return number
        case let .bool(bool): return bool
        case .null: return NSNull()
        }
    }
}

struct MCPServer: Decodable {
    struct Tool: Decodable {
        var name: String
        var description: String?
        var hidden: Bool
    }
    struct Status: Decodable {
        var state: String
        var detail: String
    }
    var name: String
    var id: String
    var transport: String?
    var enabled: Bool
    var config: MCPJSON
    var problem: String?
    var status: Status?
    var signsIn: Bool
    var signedIn: Bool
    var toolCount: Int?
    var tools: [Tool]?

    enum CodingKeys: String, CodingKey {
        case name, id, transport, enabled, config, problem, status, tools
        case signsIn = "signs_in", signedIn = "signed_in", toolCount = "tool_count"
    }
}

struct MCPServerList: Decodable {
    var path: String
    var error: String?
    var servers: [MCPServer]
}

struct MCPServerDetail: Decodable {
    var path: String
    var server: MCPServer
}

struct MCPParsedServers: Decodable {
    struct Entry: Decodable {
        var name: String?
        var config: MCPJSON
        var problem: String?
    }
    var servers: [Entry]
}
