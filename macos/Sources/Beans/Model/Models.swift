import AppKit
import Foundation

// MARK: - Device

struct ProviderCredential: Hashable, Identifiable {
    enum Kind: Hashable {
        case deepseek
        case anthropic
        case opencode
        case opencodeGo
        case chatgpt
        case grok
        /// A provider the user added: any server that speaks OpenAI's or Anthropic's API. Its
        /// wire value is `custom:` and a slug of the name it was added with.
        case custom(String)

        /// The providers Beans has built in, in the order the CLI lists them.
        static let builtIn: [Kind] = [.deepseek, .anthropic, .opencode, .opencodeGo, .chatgpt, .grok]

        static let customPrefix = "custom:"

        var isCustom: Bool {
            if case .custom = self { true } else { false }
        }

        /// The name people know the provider by. A custom provider's is the one the user gave it,
        /// from the account's credentials, and its slug once it is deleted.
        @MainActor var name: String {
            switch self {
            case .deepseek: "DeepSeek"
            case .anthropic: "Anthropic"
            case .opencode: "OpenCode Zen"
            case .opencodeGo: "OpenCode Go"
            case .chatgpt: "ChatGPT"
            case .grok: "Grok"
            case .custom(let wire): AppStore.shared.credential(for: self)?.name ?? String(wire.dropFirst(Self.customPrefix.count))
            }
        }

        var symbolName: String {
            switch self {
            case .deepseek, .anthropic, .opencode, .opencodeGo: "key.fill"
            case .chatgpt, .grok: "person.badge.key.fill"
            case .custom: "server.rack"
            }
        }

        var subtitle: String {
            switch self {
            case .deepseek, .anthropic, .opencode, .opencodeGo: L("API key")
            case .chatgpt, .grok: L("Subscription")
            case .custom: L("Custom")
            }
        }

        /// Connects with a pasted API key; ChatGPT and Grok sign in through the browser instead,
        /// and a custom provider is set up in its own sheet.
        var usesAPIKey: Bool {
            switch self {
            case .deepseek, .anthropic, .opencode, .opencodeGo: true
            case .chatgpt, .grok, .custom: false
            }
        }

        /// What the sign-in needs, for the subscription providers.
        var signInRequirement: String {
            switch self {
            case .chatgpt: L("It needs a ChatGPT subscription.")
            case .grok: L("It needs a SuperGrok or X Premium+ subscription.")
            case .deepseek, .anthropic, .opencode, .opencodeGo, .custom: ""
            }
        }

        /// The API root the CLI calls unless the credential names another.
        var defaultBaseURL: String {
            switch self {
            case .deepseek: "https://api.deepseek.com"
            case .anthropic: "https://api.anthropic.com"
            case .opencode: "https://opencode.ai/zen"
            case .opencodeGo: "https://opencode.ai/zen/go"
            case .chatgpt, .grok, .custom: ""
            }
        }

        /// Placeholder for the key field, naming where the key comes from.
        var keyPlaceholder: String {
            switch self {
            case .deepseek: L("sk-… from platform.deepseek.com")
            case .anthropic: L("sk-ant-… from console.anthropic.com")
            case .opencode, .opencodeGo: L("API key from opencode.ai/auth")
            case .custom: L("Optional for a server on your network")
            case .chatgpt, .grok: ""
            }
        }

        /// The identifier the CLI uses on the wire.
        var wireValue: String {
            switch self {
            case .deepseek: "deepseek"
            case .anthropic: "anthropic"
            case .opencode: "opencode"
            case .opencodeGo: "opencode-go"
            case .chatgpt: "chatgpt"
            case .grok: "grok"
            case .custom(let wire): wire
            }
        }

        init?(wireValue: String) {
            switch wireValue {
            case "deepseek": self = .deepseek
            case "anthropic": self = .anthropic
            case "opencode": self = .opencode
            case "opencode-go": self = .opencodeGo
            case "chatgpt": self = .chatgpt
            case "grok": self = .grok
            case let wire where wire.hasPrefix(Self.customPrefix): self = .custom(wire)
            default: return nil
            }
        }

        /// Provider connect methods use underscores even when the stored provider id has a
        /// hyphen.
        var connectMethodSuffix: String {
            switch self {
            case .opencodeGo: "opencode_go"
            case .custom: "custom"
            default: wireValue
            }
        }

        static func thinkingLabel(_ level: String) -> String {
            switch level {
            case "off": L("Off")
            case "minimal": L("Minimal")
            case "low": L("Low")
            case "medium": L("Medium")
            case "high": L("High")
            case "xhigh": L("Extra high")
            case "max": L("Max")
            default: level.prefix(1).uppercased() + level.dropFirst()
            }
        }
    }

    var id: Kind { kind }
    var kind: Kind
    var isConnected: Bool
    var detail: String
    /// A custom API root, when the account credential has one.
    var baseURL: String? = nil
    /// A custom provider's name, the protocol its server speaks, and the models it offers.
    var name: String? = nil
    var api: CustomAPI? = nil
    var integration: String? = nil
    var capabilities: CustomCapabilities? = nil
    var models: [CustomModel] = []
}

struct CustomCapabilities: Codable, Hashable {
    var contextWindow: UInt64? = nil
    var images: Bool? = nil
    var tools: Bool? = nil

    var params: [String: Any] {
        var value: [String: Any] = [:]
        if let contextWindow { value["context_window"] = contextWindow }
        if let images { value["images"] = images }
        if let tools { value["tools"] = tools }
        return value
    }
}

/// The wire protocol a custom provider's server speaks.
enum CustomAPI: String, CaseIterable, Hashable {
    case chatCompletions = "chat-completions"
    case responses
    case messages

    /// Product names, the same in every language.
    var title: String {
        switch self {
        case .chatCompletions: "OpenAI Chat Completions"
        case .responses: "OpenAI Responses"
        case .messages: "Anthropic Messages"
        }
    }

    /// What the CLI adds to the base URL for a model call.
    var path: String {
        switch self {
        case .chatCompletions: "/chat/completions"
        case .responses: "/responses"
        case .messages: "/v1/messages"
        }
    }

    var baseURLPlaceholder: String {
        self == .messages ? "https://api.example.com" : "https://api.example.com/v1"
    }

    /// The URL the CLI calls for a base URL as typed: a pasted endpoint is cut back to its root
    /// first, as the CLI does, then this protocol's path goes on.
    func endpoint(for baseURL: String) -> String {
        var root = baseURL.trimmingCharacters(in: .whitespacesAndNewlines)
        while root.hasSuffix("/") { root.removeLast() }
        let pasted: [String] =
            switch self {
            case .chatCompletions: ["/chat/completions", "/models"]
            case .responses: ["/responses", "/models"]
            case .messages: ["/v1/messages", "/v1/models", "/v1", "/messages", "/models"]
            }
        if let suffix = pasted.first(where: { root.hasSuffix($0) }) { root.removeLast(suffix.count) }
        return root + path
    }
}

/// A model a custom provider offers, with what its server's model list says of it and the
/// thinking levels the CLI says it takes.
struct CustomModel: Hashable {
    var id: String
    var name: String? = nil
    var contextWindow: Int? = nil
    /// Whether it takes images.
    var images: Bool? = nil
    var levels: [String] = []
    var maxOutput: Int? = nil
    var reasoning: Bool? = nil
    var tools: Bool? = nil
    var thinkingFormat: String? = nil
    var thinkingCanDisable: Bool? = nil

    var displayName: String { name ?? id }

    func merging(_ discovered: CustomModel) -> CustomModel {
        var model = self
        model.name = discovered.name ?? name
        model.contextWindow = discovered.contextWindow ?? contextWindow
        model.maxOutput = discovered.maxOutput ?? maxOutput
        model.images = discovered.images ?? images
        model.reasoning = discovered.reasoning ?? reasoning
        model.tools = discovered.tools ?? tools
        model.thinkingFormat = discovered.thinkingFormat ?? thinkingFormat
        model.thinkingCanDisable = discovered.thinkingCanDisable ?? thinkingCanDisable
        return model
    }
}

/// A server people often add: its API, its base URL, and where its key comes from.
struct CustomProviderPreset: Hashable {
    let name: String
    let api: CustomAPI
    let baseURL: String
    let keyPlaceholder: String
    var compatible: Bool = false

    /// Services in the cloud, for the Add Provider menu.
    static var cloud: [CustomProviderPreset] {
        [
            CustomProviderPreset(name: "OpenAI Compatible", api: .chatCompletions, baseURL: "",
                keyPlaceholder: L("Optional API key"), compatible: true),
            CustomProviderPreset(name: "Anthropic Compatible", api: .messages, baseURL: "",
                keyPlaceholder: L("Optional API key"), compatible: true),
            CustomProviderPreset(
                name: "OpenAI", api: .responses, baseURL: "https://api.openai.com/v1",
                keyPlaceholder: L("sk-… from platform.openai.com")),
            CustomProviderPreset(
                name: "OpenRouter", api: .chatCompletions, baseURL: "https://openrouter.ai/api/v1",
                keyPlaceholder: L("sk-or-… from openrouter.ai/keys")),
            CustomProviderPreset(
                name: "Gemini", api: .chatCompletions, baseURL: "https://generativelanguage.googleapis.com/v1beta/openai",
                keyPlaceholder: L("Key from aistudio.google.com")),
            CustomProviderPreset(
                name: "Groq", api: .chatCompletions, baseURL: "https://api.groq.com/openai/v1",
                keyPlaceholder: L("gsk_… from console.groq.com")),
            CustomProviderPreset(
                name: "Together AI", api: .chatCompletions, baseURL: "https://api.together.xyz/v1",
                keyPlaceholder: L("Key from api.together.ai")),
        ]
    }

    /// The preset for a base URL as typed, by its host and port, so a URL pasted into an empty
    /// sheet still finds the server's name and key hint.
    static func matching(_ baseURL: String) -> CustomProviderPreset? {
        guard let url = URL(string: baseURL.trimmingCharacters(in: .whitespacesAndNewlines)), let host = url.host else { return nil }
        return (cloud + local).first { preset in
            let known = URL(string: preset.baseURL)
            return known?.host == host && known?.port == url.port
        }
    }

    /// Model servers that run on the user's own computers.
    static var local: [CustomProviderPreset] {
        [
            CustomProviderPreset(name: L("Local server"), api: .chatCompletions, baseURL: "",
                keyPlaceholder: L("Optional API key"), compatible: true),
            CustomProviderPreset(
                name: "Ollama", api: .chatCompletions, baseURL: "http://localhost:11434/v1",
                keyPlaceholder: L("Optional for a server on your network")),
            CustomProviderPreset(
                name: "LM Studio", api: .chatCompletions, baseURL: "http://localhost:1234/v1",
                keyPlaceholder: L("Optional for a server on your network")),
        ]
    }
}

/// The custom provider sheet's model list: the models the server listed and the ones the user
/// added or saved, which of them bots can pick, and the default.
enum ModelChecklist {
    /// Takes a new listing: the models to keep (picked or added by hand) stay where they are
    /// with the listing's facts, the rest of the old listing goes, and the new one follows.
    static func merge(_ models: [CustomModel], keeping keep: (String) -> Bool, with listed: [CustomModel]) -> [CustomModel] {
        let facts = Dictionary(listed.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        var merged = models.filter { keep($0.id) }.map { model in facts[model.id].map { model.merging($0) } ?? model }
        var seen = Set(merged.map(\.id))
        for model in listed where seen.insert(model.id).inserted { merged.append(model) }
        return merged
    }

    /// The models whose name or id holds the query.
    static func filter(_ models: [CustomModel], _ query: String) -> [CustomModel] {
        guard !query.isEmpty else { return models }
        return models.filter { $0.id.localizedCaseInsensitiveContains(query) || ($0.name?.localizedCaseInsensitiveContains(query) ?? false) }
    }

    /// The id an Add row offers for a query: one no model has yet.
    static func addCandidate(_ query: String, in models: [CustomModel]) -> String? {
        let id = query.trimmingCharacters(in: .whitespacesAndNewlines)
        return id.isEmpty || models.contains { $0.id == id } ? nil : id
    }

    /// The picked ids as the CLI keeps them: the default first, then the others in list order.
    static func orderedIDs(_ models: [CustomModel], selected: Set<String>, defaultID: String?) -> [String] {
        let picked = models.map(\.id).filter { selected.contains($0) }
        guard let defaultID, picked.contains(defaultID) else { return picked }
        return [defaultID] + picked.filter { $0 != defaultID }
    }
}

/// A model the CLI's catalog offers, as the snapshot names it, and the thinking levels it
/// takes, lowest first.
struct ProviderModel: Hashable {
    var provider: ProviderCredential.Kind
    var id: String
    var label: String
    var levels: [String]

    /// Every thinking level, lowest first.
    private static let allThinkingLevels = ["off", "minimal", "low", "medium", "high", "xhigh", "max"]

    /// The thinking levels `model` takes among `models`, one provider's in the catalog's order:
    /// the default model's when it is nil, and every level they take for a model the catalog
    /// does not have. nil on a bot means the model's default.
    static func thinkingLevels(for model: String?, among models: [ProviderModel]) -> [(id: String, label: String)] {
        let known = models.first { $0.id == (model ?? models.first?.id) }
        let ids = known?.levels ?? allThinkingLevels.filter { level in models.contains { $0.levels.contains(level) } }
        return ids.map { ($0, ProviderCredential.Kind.thinkingLabel($0)) }
    }
}

/// A paired machine or phone. Its `os` decides whether it is a Runner: only desktop
/// systems run the CLI, hold provider credentials, and get bots assigned.
struct Device: Identifiable, Hashable {
    enum OS: String, Hashable, CaseIterable {
        case macos
        case linux
        case windows
        case ios
        case ipados
        case android
        /// A machine the relay lists that never said what it is: never a Runner.
        case unknown

        var displayName: String {
            switch self {
            case .macos: "macOS"
            case .linux: "Linux"
            case .windows: "Windows"
            case .ios: "iOS"
            case .ipados: "iPadOS"
            case .android: "Android"
            case .unknown: ""
            }
        }

        /// Desktop systems run the agent loop. Phones and tablets never do.
        var isDesktop: Bool {
            switch self {
            case .macos, .linux, .windows: true
            case .ios, .ipados, .android, .unknown: false
            }
        }
    }

    enum Status: Hashable {
        case online
        case offline
        case pairing

        var label: String {
            switch self {
            case .online: L("Online")
            case .offline: L("Offline")
            case .pairing: L("Pairing", context: "device state")
            }
        }
    }

    let id: String
    var name: String
    var model: String
    var os: OS
    var osVersion: String
    var isThisDevice: Bool
    var status: Status
    var lastSeen: Date
    var machineKey: String
    /// Plugins installed on this Runner, as it advertises them. Secrets stay on the Runner.
    var plugins: [InstalledPlugin] = []
    /// The `beans` this Device runs.
    var version: String = ""
    /// Set for a CLI that updates itself (one installed with the site's script); a CLI an app
    /// carries updates with the app.
    var update: CLIUpdate? = nil
    /// Original machine-reported name, independent of the account alias.
    var machineName: String = ""

    /// A self-updating CLI's updates, as its `machine` blob says.
    struct CLIUpdate: Hashable {
        var auto: Bool
        /// A newer release than `version`, when the last check found one.
        var latest: String?
        /// `installing`, `restarting` (waits for its bots to finish), or `installed` (restart
        /// beans serve by hand); nil otherwise.
        var state: String?
        var error: String?
    }

    /// Derived from `os` alone: a desktop Device is a Runner and can be assigned bots.
    var isRunner: Bool { os.isDesktop }

    var roleLabel: String { isRunner ? L("Runner") : L("Device") }

    /// What the Device panes say about a machine that never said what it is.
    static var unknownNote: String {
        L("This machine is paired to your account but has not sent its name or system. If you don't recognize it, unpair it.")
    }
}

// MARK: - Bot

/// nil plugins allows every installed plugin; an empty list allows none.
struct BotCapabilities: Hashable {
    var shell = true
    var write = true
    var plugins: [String]? = nil
}

/// Kept independently from fork capabilities. The CLI enforces both policies.
struct BotAccess: Decodable, Hashable {
    struct Connection: Decodable, Hashable {
        var capabilities: Set<String>
        var tools: Set<String>?
    }
    var connections: [String: Connection]?
    var filesystem: String?
    var shell: Bool?
    var drafts: Bool?
}

struct SecretRequest: Decodable, Hashable {
    struct Field: Decodable, Hashable {
        var name: String
        var label: String
    }
    var `use`: String
    var site: String?
    var fields: [Field]
}

struct Bot: Identifiable, Hashable {
    let id: String
    var name: String
    /// What the bot does and how it should work.
    var description: String
    var symbolName: String
    var accent: Accent
    var runnerID: Device.ID
    var provider: ProviderCredential.Kind
    /// nil means the provider's default model.
    var model: String? = nil
    /// How much the model thinks; nil means the provider's default.
    var thinking: String? = nil
    /// A custom profile image, kept as a `file` blob like a message attachment. Shown in place
    /// of the generated portrait once this computer has the bytes.
    var avatar: Attachment? = nil
    /// The generated portrait's saved appearance; nil keeps the bot-ID seeded default. Kept
    /// independently of `avatar`, which wins while it exists.
    var look: BotLook? = nil
    var shape: String? = nil
    var workdir: String? = nil
    var permissions: BotAccess? = nil
    var capabilities = BotCapabilities()
    var createdAt: Date
}

// MARK: - Auto-review

/// One Auto-review rule: what a bot wants to do, in the user's words, and whether that runs on
/// its own or asks first. Always allow on a shell command adds the rule Auto-review proposed; on
/// a plugin tool it adds a rule for that exact tool.
struct AutoReviewRule: Hashable, Identifiable {
    enum Behavior: String, Hashable {
        case allow
        case ask

        var title: String { self == .allow ? L("Allow automatically") : L("Ask first") }
    }

    var id: String
    var text: String
    var behavior: Behavior
    /// The exact plugin tool (`github/create_issue`) Always allow saved this rule for.
    var tool: String? = nil
}

/// The check on effectful plugin actions and shell commands, shared by every Device through
/// the roster: on, a small model on the bot's provider asks only when needed; off, each one asks.
struct AutoReview: Hashable {
    var isEnabled: Bool = true
    var rules: [AutoReviewRule] = []
    var provider: String? = nil
    var models: [String: String] = [:]
}

// MARK: - Plugins

/// A plugin as its Runner advertises it: installed, and in what state.
struct InstalledPlugin: Identifiable, Hashable {
    enum State: String, Hashable {
        case ready
        case needsSetup = "needs_setup"
        case needsAuth = "needs_auth"
        case connecting
        case error
        case unknown
    }

    let id: String
    var name: String
    var description: String
    var version: String
    var icon: String
    var state: State
    var detail: String
    /// `mcp.json` for one of the Runner's own MCP servers, which the server sheet edits.
    var source: String? = nil

    var symbolName: String { icon.isEmpty ? "puzzlepiece.extension" : icon }
    var isMcpServer: Bool { source == "mcp.json" }

    var stateColor: NSColor {
        switch state {
        case .ready: .systemGreen
        case .connecting: .controlAccentColor
        case .error: .systemRed
        case .needsSetup, .needsAuth, .unknown: .systemOrange
        }
    }
}

/// A marketplace plugin, with the Runners that already have it.
struct MarketplacePlugin: Identifiable, Hashable {
    struct Server: Hashable {
        var name: String
        /// The URL of a remote server, or the command a local one runs on the Runner.
        var address: String
        var isRemote: Bool
        var signsIn: Bool
    }

    struct Skill: Hashable {
        var name: String
        var description: String
    }

    struct Variable: Hashable {
        var name: String
        var description: String
        var secret: Bool
        var required: Bool
    }

    let id: String
    var name: String
    var description: String
    var icon: String
    var homepage: String?
    /// Who makes it.
    var author: String
    var category: String
    var isFeatured: Bool
    var tags: [String]
    var servers: [Server]
    var skills: [Skill]
    var variables: [Variable]
    var installedOn: [Device.ID]

    var symbolName: String { icon.isEmpty ? "puzzlepiece.extension" : icon }
    /// At least one server signs in with OAuth on the Runner.
    var signsIn: Bool { servers.contains { $0.signsIn } }
}

/// A bot to add from the marketplace: the profile it starts with, the plugins it works with,
/// the routines it brings (added paused), and facts it starts out knowing.
struct BotTemplate: Identifiable, Hashable {
    struct Routine: Hashable {
        var name: String
        /// The CLI's sentence for the schedule, such as "Weekdays at 9:00 AM".
        var scheduleText: String
        var prompt: String
    }

    let id: String
    var name: String
    /// One line for the marketplace's rows.
    var summary: String
    /// What the bot does and how it should work: the new bot's description.
    var description: String
    var symbolName: String
    var accent: Accent
    var category: String
    var isFeatured: Bool
    var author: String
    var plugins: [MarketplacePlugin.ID]
    var routines: [Routine]
    var memory: [String]
}

/// What the marketplace offers, in the index's order.
struct Marketplace {
    var plugins: [MarketplacePlugin] = []
    var bots: [BotTemplate] = []
}

/// One installed plugin in full, as its Runner reports it: never a secret's value.
struct PluginDetail {
    struct Variable: Hashable {
        var name: String
        var description: String
        var secret: Bool
        var required: Bool
        var isSet: Bool
        var value: String?
    }

    struct Server: Hashable {
        var name: String
        var kind: String
        var url: String?
        var oauth: Bool
        var signedIn: Bool
        /// While a device-flow sign-in waits: the code to enter, and the page to enter it on.
        var code: String? = nil
        var link: String? = nil
    }

    var status: InstalledPlugin
    var homepage: String?
    var variables: [Variable]
    var servers: [Server]
    var skills: [(name: String, description: String)]
}

/// A bot asking before a plugin or shell action runs, or before a plugin is installed.
struct PermissionRequest: Hashable {
    enum Decision: String, Hashable {
        case pending
        case allowed
        case always
        case denied
        case expired
        /// The user sent a new message instead of answering.
        case dismissed
        /// A sign-in card: the flow finished.
        case connected
        case failed
    }

    var secret: SecretRequest? = nil
    var pluginID: String
    var pluginName: String
    var tool: String
    var summary: String
    var decision: Decision
    /// A sign-in mid-flow: where to go and the code to enter there.
    var link: String? = nil
    var code: String? = nil
    /// Why Auto-review paused the action, when it did.
    var reason: String? = nil
    /// The rule Always allow adds, which Auto-review proposed for a shell command.
    var rule: String? = nil
    /// A shell card's whole command, where `summary` is its first line.
    var command: String? = nil
    /// Draft metadata; the Runner writes only after approval.
    var title: String? = nil
    var content: String? = nil
    var path: String? = nil

    /// The command as the card and its sheet show it, without the summary's `$ ` prompt.
    var fullCommand: String {
        command ?? (summary.hasPrefix("$ ") ? String(summary.dropFirst(2)) : summary)
    }

    var isPending: Bool { decision == .pending }
    var isInstall: Bool { tool == "install" }
    /// A shell command on the bot's Runner.
    var isShell: Bool { pluginID == "computer" }
    /// A sign-in card: Sign in starts the OAuth flow on the Runner.
    var isConnect: Bool { tool == "connect" }
    var isProposal: Bool { tool == "propose" }

    /// "wants to use GitHub" / "wants to install GitHub" / "needs a sign-in to GitHub" /
    /// "wants to run a command on Workbench"
    var verbPhrase: String {
        if isProposal { return L("proposes a draft") }
        if isConnect { return L("needs a sign-in to %@", pluginName) }
        if isShell { return L("wants to run a command on %@", pluginName) }
        return isInstall ? L("wants to install %@", pluginName) : L("wants to use %@", pluginName)
    }

    var decisionText: String {
        if isProposal {
            switch decision {
            case .pending: return L("Waiting for review")
            case .allowed: return path == nil ? L("Approved; not saved") : L("Approved and saved")
            case .denied: return L("Declined")
            case .expired: return L("No answer in time")
            case .dismissed: return L("Dismissed")
            case .failed: return L("Save failed")
            default: return L("Not saved")
            }
        }
        return switch decision {
        case .pending: L("Waiting for you")
        case .allowed: isConnect ? L("Signing in") : L("Allowed once")
        case .always: L("Always allowed")
        case .denied: isConnect ? L("Not now") : L("Denied")
        case .expired: L("No answer in time")
        case .dismissed: L("Dismissed")
        case .connected: L("Signed in")
        case .failed: L("Sign-in failed")
        }
    }

    /// The buttons a pending card offers: (title, decision). A shell command offers Always
    /// allow only with a rule to add.
    var choices: [(String, String)] {
        if isProposal { return [(L("Approve & save"), "allow"), (L("Decline"), "deny")] }
        if isConnect { return [(L("Sign in"), "allow"), (L("Not now"), "deny")] }
        if isInstall { return [(L("Allow"), "allow"), (L("Deny"), "deny")] }
        if isShell && rule == nil { return [(L("Allow once"), "allow"), (L("Deny"), "deny")] }
        return [(L("Allow once"), "allow"), (L("Always allow"), "always"), (L("Deny"), "deny")]
    }
}

/// A bot's memory as its Runner reports it: the curated index with its load budget, and the
/// other files by name. `here` is false when the bot runs elsewhere and only `runner` is known.
struct BotMemory: Hashable {
    var botID: Bot.ID
    var here: Bool
    var runner: String
    var path: String
    var text: String
    var hash: String
    var lines: Int
    var bytes: Int
    var truncated: Bool
    var maxLines: Int
    var maxBytes: Int
    var topics: [String]
    var logs: [String]

    /// "12 lines · 1.2 KB of 24 KB", or "Empty".
    var budgetSummary: String {
        guard lines > 0 else { return L("Empty") }
        return lines == 1
            ? L("%d line · %@ of %@", lines, Format.kilobytes(bytes), Format.kilobytes(maxBytes))
            : L("%d lines · %@ of %@", lines, Format.kilobytes(bytes), Format.kilobytes(maxBytes))
    }

    /// "2 topics · 5 days of logs"
    var filesSummary: String {
        var parts: [String] = []
        if !topics.isEmpty { parts.append(topics.count == 1 ? L("%d topic", topics.count) : L("%d topics", topics.count)) }
        if !logs.isEmpty { parts.append(logs.count == 1 ? L("%d day of logs", logs.count) : L("%d days of logs", logs.count)) }
        return parts.isEmpty ? L("No other notes yet") : parts.joined(separator: " · ")
    }
}

// MARK: - Routine

/// A recurring task a bot runs on a schedule in its direct chat, as the roster carries it. The
/// schedule's words, the next run, and the running state come from the CLI.
struct Routine: Identifiable, Hashable {
    let id: String
    var botID: Bot.ID
    var name: String
    /// The task, written to the bot, handed to it on every run.
    var prompt: String
    /// `every 30m`, `every 2h`, `every 1d`, or five cron fields in the Runner's local time.
    var schedule: String
    /// The schedule in words: "Weekdays at 9:00 AM".
    var scheduleText: String
    var isEnabled: Bool
    /// Why Beans paused it, when it did: "away".
    var pausedReason: String?
    var lastRunAt: Date?
    /// How the last run ended: "sent", "pass", or "error".
    var lastOutcome: String?
    var nextRunAt: Date?
    var isRunning: Bool
    var createdAt: Date
    /// The script the Runner runs at each due time before the bot does; the bot runs only when
    /// it finds something. `nextRunAt` is then the next check.
    var check: String? = nil

    /// The line under the name in the inspector: the schedule, then what is going on.
    var detail: String {
        if isRunning { return L("%@ · Running…", scheduleText) }
        guard isEnabled else { return pausedReason == "away" ? L("%@ · Paused while you were away", scheduleText) : L("%@ · Paused", scheduleText) }
        if let nextRunAt {
            return check == nil ? L("%@ · Next %@", scheduleText, Format.upcoming(nextRunAt)) : L("%@ · Next check %@", scheduleText, Format.upcoming(nextRunAt))
        }
        return scheduleText
    }

    /// "Today 9:00 AM · replied", "Never", "Yesterday 6:00 PM · nothing to report".
    var lastRunSummary: String {
        guard let lastRunAt else { return L("Never") }
        let when = Format.daySeparator(lastRunAt)
        switch lastOutcome {
        case "sent": return L("%@ · replied", when)
        case "pass": return L("%@ · nothing to report", when)
        case "error": return L("%@ · failed", when)
        default: return when
        }
    }
}

// MARK: - Message

struct ToolInvocation: Hashable {
    var name: String
    var summary: String
    var detail: String
    var isRunning: Bool
    /// What the call does, in the bot's words: a shell command's "Install dependencies".
    var description: String?
    /// The command currently running inside a codemode script, distinct from its plugin.
    var scriptCommand: String? = nil
    /// The bot a message_bot call goes to.
    var targetBotID: Bot.ID?
    /// A shell command's card, which the transcript shows only while the command needs the user
    /// (`isShown`). Every `bash` row has one.
    var run: CommandRun? = nil
    var targetAgentID: String? = nil

    /// A finished message_bot call: the one tool the transcript shows, as "Messaged ◉ Name".
    /// Everything else a bot does with tools stays behind the "is working" row.
    var isSentMessage: Bool {
        name == "message_bot" && !isRunning && summary.hasPrefix("Messaged ")
    }

    /// Whether the transcript shows the row: a sent message's marker, or a command's card while
    /// the command needs the user. That is while Auto-review asks to run it, and once the bot
    /// handed the running command over (its turn ended, or it waits on the command at a
    /// question) until it ends. Before that the bot deals with it, and the command shows only
    /// as the working row's activity.
    var isShown: Bool {
        guard let run else { return isSentMessage }
        return run.state == .asking || (run.isLive && run.handedOver)
    }

    var symbolName: String {
        switch name {
        case "message_bot": "arrow.triangle.turn.up.right.diamond.fill"
        case "list_teammates": "person.2.fill"
        case "create_bot": "person.badge.plus"
        case "edit_bot": "person.text.rectangle"
        case "read": "doc.text"
        case "write": "square.and.pencil"
        case "edit": "pencil.line"
        case "bash": "terminal"
        case "grep": "text.magnifyingglass"
        case "find": "magnifyingglass"
        case "ls": "folder"
        default: "wrench.and.screwdriver.fill"
        }
    }
}

/// Where a shell command stands: Auto-review checking it, the question it asks, the command
/// running in its terminal, what the command asks, and that it ended. While it asks, its card
/// takes the answer to the question (`chats.permission`); once the bot handed the running
/// command over, the user's answer to it (`bash.stdin`) and a Stop (`bash.stop`).
struct CommandRun: Hashable {
    enum State: String, Hashable {
        /// Auto-review is judging it.
        case checking
        /// For the user's permission.
        case asking
        /// Running: printing, or quiet.
        case running
        /// Running, at a question the user can read.
        case waiting
        case exited
        /// Exited with a nonzero code.
        case failed
        case stopped
        /// Not allowed, so it never ran.
        case denied
        /// Nobody answered the question in time.
        case expired
        /// The user sent a new message instead of answering, so it never ran.
        case dismissed
    }

    /// The terminal session running it, once one does. Nil before it starts, and on a Windows
    /// Runner, where a command runs on pipes and takes no answers.
    var sessionID: String?
    /// The command, its first 8,000 characters.
    var command: String
    var state: State
    /// The line it asks with: "[sudo] password for ana:".
    var prompt: String?
    /// Its last lines, as the bottom of a terminal shows them. Never what was typed.
    var output: String?
    /// The Runner it runs on, for the question: "Workbench".
    var device: String?
    /// Why Auto-review asked.
    var reason: String?
    /// The rule Always allow adds.
    var rule: String?
    /// The bot left the command to the user: its turn ended with the command still running, or
    /// it waits on the command at a question.
    var handedOver = false
    /// It runs in the background: the bot started it there, or the user sent it. Stop in the chat
    /// leaves it running.
    var background = false

    var isLive: Bool { state == .waiting || state == .running }
    /// The command runs in a session here or on its Runner: it takes answers and a Stop.
    var takesInput: Bool { isLive && sessionID != nil }
    /// It ran and ended: by itself, with a nonzero code, or stopped.
    var hasEnded: Bool { state == .exited || state == .failed || state == .stopped }

    /// The command's first line with anything on it.
    var firstLine: String {
        command.split(separator: "\n").map { $0.trimmingCharacters(in: .whitespaces) }.first { !$0.isEmpty } ?? command
    }

    /// Whether what the user types should show as they type it: a yes-or-no question. Anything
    /// else may be a secret.
    var asksYesOrNo: Bool {
        guard let prompt = prompt?.lowercased() else { return false }
        return prompt.contains("[y/n]") || prompt.contains("(y/n)") || prompt.contains("(yes/no")
    }

    /// The buttons the question offers: (title, decision). Always allow only with a rule to add.
    var choices: [(String, String)] {
        rule == nil ? [(L("Allow once"), "allow"), (L("Deny"), "deny")] : [(L("Allow once"), "allow"), (L("Always allow"), "always"), (L("Deny"), "deny")]
    }
}

/// A file sent with a message. The bytes live under the CLI's data directory once this Device
/// has them; `width` and `height` size an image's thumbnail before the file arrives.
struct Attachment: Hashable, Identifiable {
    let id: String
    var name: String
    var mime: String
    var size: Int
    var width: Int?
    var height: Int?

    var isImage: Bool { mime.hasPrefix("image/") }

    /// "Photo", "3 photos", "report.pdf", "2 files": the preview of a message with no text.
    static func summary(_ attachments: [Attachment]) -> String {
        guard let first = attachments.first else { return "" }
        if attachments.count == 1 { return first.isImage ? L("Photo") : first.name }
        return attachments.allSatisfy(\.isImage) ? L("%d photos", attachments.count) : L("%d files", attachments.count)
    }

    static func sizeText(_ bytes: Int) -> String {
        if bytes < 1024 { return "\(bytes) B" }
        if bytes < 1024 * 1024 { return "\(Int((Double(bytes) / 1024).rounded())) KB" }
        return String(format: "%.1f MB", Double(bytes) / 1024 / 1024)
    }
}

struct Message: Identifiable, Hashable {
    enum Author: Hashable {
        case you
        case bot(Bot.ID)
        case system

        var botID: Bot.ID? {
            if case let .bot(id) = self { return id }
            return nil
        }

        var isYou: Bool { self == .you }
    }

    enum Body: Hashable {
        case text(String)
        case tool(ToolInvocation)
        case handoff(from: Bot.ID, to: Bot.ID, reason: String)
        case notice(String)
        case permission(PermissionRequest)
    }

    enum State: Hashable {
        case thinking
        case streaming
        case complete
        case failed(String)
    }

    let id: String
    var author: Author
    var body: Body
    var state: State
    var createdAt: Date
    /// Files sent with a text body; other bodies carry none.
    var attachments: [Attachment]
    /// The message the user answers with this one, quoted.
    var replyTo: ReplyQuote?
    /// A message of the user's the bot's turn holds for its next step; Send now has it read now.
    var reviewID: String? = nil
    var reviewVersion: UInt64? = nil
    var reviewState: String? = nil
    var queued = false

    init(
        id: String = "msg-\(UUID().uuidString.lowercased())",
        author: Author,
        body: Body,
        state: State = .complete,
        createdAt: Date = Date(),
        attachments: [Attachment] = [],
        replyTo: ReplyQuote? = nil
    ) {
        self.id = id
        self.author = author
        self.body = body
        self.state = state
        self.createdAt = createdAt
        self.attachments = attachments
        self.replyTo = replyTo
    }

    /// A finished text message, the user's or a bot's, which a reply can answer.
    var canBeQuoted: Bool {
        guard case .text = body else { return false }
        return state == .complete && author != .system
    }

    /// A `bash` row's command.
    var commandRun: CommandRun? {
        if case let .tool(tool) = body { return tool.run }
        return nil
    }

    /// A command running in its terminal that the bot's call still waits on: Run in Background
    /// sends it there, and the call returns.
    var runsInForeground: Bool {
        guard case let .tool(tool) = body, let run = tool.run else { return false }
        return tool.isRunning && run.takesInput && !run.background
    }

    var text: String {
        switch body {
        case let .text(value): value
        case let .tool(tool): tool.summary
        case let .handoff(_, _, reason): reason
        case let .notice(value): value
        case let .permission(request): request.summary
        }
    }

    var isTranscriptText: Bool {
        if case .text = body { return true }
        return false
    }
}

/// A message quoted by the user's reply: who wrote it and how it opens, as the CLI keeps it
/// with the reply, so the quote reads the same where the original has not loaded.
struct ReplyQuote: Hashable {
    let messageID: Message.ID
    let author: Message.Author
    let text: String

    /// The quote of `message` the CLI makes, for a reply it has not confirmed yet: its words
    /// without the Markdown, on one line.
    @MainActor
    init?(quoting message: Message) {
        guard message.canBeQuoted else { return nil }
        let words = RenderedMessage(message.text, textColor: .labelColor).plainText
        var line = words.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        if line.isEmpty { line = message.attachments.map(\.name).joined(separator: ", ") }
        if line.count > 280 { line = String(line.prefix(280)).trimmingCharacters(in: .whitespaces) + "…" }
        self.init(messageID: message.id, author: message.author, text: line)
    }

    init(messageID: Message.ID, author: Message.Author, text: String) {
        self.messageID = messageID
        self.author = author
        self.text = text
    }
}

// MARK: - Chat

struct Chat: Identifiable, Hashable {
    /// A DM is a fixed one-to-one thread with a single bot. A group holds one to six bots and
    /// can gain or lose members after it is created.
    enum Kind: String, Hashable {
        case dm
        case group
    }

    static let maxGroupBots = 6

    let id: String
    var kind: Kind
    var customTitle: String?
    var botIDs: [Bot.ID]
    var messages: [Message]
    var unreadCount: Int
    var isPinned: Bool
    var createdAt: Date
    /// What the turns run in this chat used, from the Runner that ran them.
    var usage: ChatUsage? = nil
    /// The CLI holds messages older than the ones here; the transcript asks for them by page.
    var hasMore = false
    /// The group member holding the work, as the CLI last said.
    var ownerBotID: Bot.ID? = nil
    /// What a group is for, which every member reads in its system prompt; empty for none.
    var groupDescription = ""

    var isGroup: Bool { kind == .group }

    /// A group's owner: the one set, else the first member, as the CLI picks.
    var owner: Bot.ID? {
        guard isGroup else { return nil }
        if let ownerBotID, botIDs.contains(ownerBotID) { return ownerBotID }
        return botIDs.first
    }
    var isDM: Bool { kind == .dm }

    /// Whether another bot may join. Only groups grow, and never past the cap.
    var canAddBot: Bool { isGroup && botIDs.count < Self.maxGroupBots }

    /// Whether a bot may leave. Groups keep at least one bot; DMs never change.
    var canRemoveBot: Bool { isGroup && botIDs.count > 1 }

    var lastActivity: Date {
        messages.last?.createdAt ?? createdAt
    }

    func index(of messageID: Message.ID) -> Int? {
        messages.firstIndex { $0.id == messageID }
    }
}


// MARK: - Usage

/// Tokens and money the turns in a chat used. `contextTokens` and `contextWindow` are the last
/// turn's; the rest accumulate.
struct ChatUsage: Hashable {
    var contextTokens: Int
    var contextEstimated: Bool = false
    var contextWindow: Int
    var inputTokens: Int
    var outputTokens: Int
    var cacheReadTokens: Int
    var costUSD: Double
    var costKnown: Bool? = nil
    var turns: Int
    var model: String

    /// "128k of 1M · 13%", or "128k" when the window is unknown.
    var contextSummary: String {
        var summary = Format.tokens(contextTokens)
        if contextWindow > 0 {
            let percent = Int((Double(contextTokens) / Double(contextWindow) * 100).rounded())
            summary = L("%@ of %@ · %d%%", summary, Format.tokens(contextWindow), percent)
        }
        return contextEstimated ? "≈ " + summary : summary
    }

    /// "$0.42 · 18 turns"
    var spendSummary: String {
        let dollars = costUSD < 0.01 && costUSD > 0 ? "<$0.01" : String(format: "$%.2f", costUSD)
        let cost = costKnown == true ? dollars : costUSD > 0 ? L("%@ known subtotal · total unknown", dollars) : L("Unknown cost")
        return turns == 1 ? L("%@ · %d turn", cost, turns) : L("%@ · %d turns", cost, turns)
    }
}

extension Format {
    /// "today 9:00 AM", "tomorrow 9:00 AM", "Monday 9:00 AM", "Oct 1 9:00 AM": when a run is due.
    static func upcoming(_ date: Date) -> String {
        let calendar = Calendar.current
        if calendar.isDateInToday(date) { return L("today %@", time(date)) }
        if calendar.isDateInTomorrow(date) { return L("tomorrow %@", time(date)) }
        if date.timeIntervalSinceNow < 60 * 60 * 24 * 6 {
            let weekday = Format.formatter("EEEE")
            return "\(weekday.string(from: date)) \(time(date))"
        }
        let day = Format.formatter("MMMd")
        return "\(day.string(from: date)) \(time(date))"
    }

    /// "1.2 KB", "24 KB"
    static func kilobytes(_ bytes: Int) -> String {
        let kb = Double(bytes) / 1000
        return kb >= 10 || kb == kb.rounded() ? "\(Int(kb.rounded())) KB" : String(format: "%.1f KB", kb)
    }

    /// "950", "12k", "1.2M"
    static func tokens(_ count: Int) -> String {
        if count >= 1_000_000 { return String(format: "%.1fM", Double(count) / 1_000_000) }
        if count >= 1_000 { return "\(count / 1_000)k" }
        return "\(count)"
    }
}

/// A naming prompt's target excludes mutable labels and presence.
struct DeviceNameTarget: Equatable {
    let id: Device.ID
    let machineKey: String
    let os: Device.OS
    init(_ device: Device) { id = device.id; machineKey = device.machineKey; os = device.os }
}

/// Connection/account epochs and membership changes revoke old naming intents.
struct DeviceNameOwnership {
    private(set) var generation: UInt64 = 0
    private var targets: [Device.ID: DeviceNameTarget] = [:]
    mutating func observe(_ event: StoreEvent, devices: [Device]) {
        let reset: Bool
        switch event {
        case .connectionChanged, .snapshotReplaced, .identityChanged: reset = true
        case .rosterChanged: reset = false
        default: return
        }
        let next = devices.reduce(into: [Device.ID: DeviceNameTarget]()) { $0[$1.id] = DeviceNameTarget($1) }
        if reset || next != targets { generation &+= 1 }
        targets = next
    }
}


extension Bot {
    /// Static upstream silhouette fallback. Saved fork looks and custom photos take precedence.
    var upstreamShapeImage: NSImage? {
        guard look == nil, let shape, let encoded = Self.upstreamMasks[shape],
              let data = Data(base64Encoded: encoded), let mask = NSImage(data: data) else { return nil }
        let image = NSImage(size: NSSize(width: 64, height: 64))
        image.lockFocus()
        mask.draw(in: NSRect(x: 0, y: 0, width: 64, height: 64))
        accent.color.setFill()
        NSRect(x: 0, y: 0, width: 64, height: 64).fill(using: .sourceAtop)
        // Eye offsets and endpoints follow desktop/characters.go, in its normalized y-down box.
        let offsets: [String: NSPoint] = ["cloud": .init(x: -0.02, y: 0.04),
            "drop": .init(x: 0.02, y: 0.06), "sparkle": .init(x: -0.22, y: 0.24),
            "clover": .init(x: -0.06, y: 0.1), "flower": .init(x: -0.08, y: 0.1),
            "heart": .init(x: -0.12, y: 0.2), "star": .init(x: -0.24, y: 0.24)]
        let offset = offsets[shape] ?? .zero
        let dx = 0.13 * sin(25 * Double.pi / 180)
        let dy = 0.13 * cos(25 * Double.pi / 180)
        NSColor.white.setStroke()
        for center in [NSPoint(x: 0.04, y: -0.2), NSPoint(x: 0.44, y: -0.3)] {
            let eye = NSBezierPath()
            eye.lineWidth = 0.17 * 32
            eye.lineCapStyle = .round
            eye.move(to: NSPoint(x: (center.x + offset.x - dx + 1) * 32,
                                 y: (1 - center.y - offset.y + dy) * 32))
            eye.line(to: NSPoint(x: (center.x + offset.x + dx + 1) * 32,
                                 y: (1 - center.y - offset.y - dy) * 32))
            eye.stroke()
        }
        image.unlockFocus()
        return image
    }

    // Exact generated masks from bf1ab709 mobile/assets/shapes, embedded to keep this adapter
    // independent of bundle-resource changes owned by another actor.
    private static let upstreamMasks: [String: String] = [
        "blob": "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",
        "pebble": "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",
        "squircle": "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",
        "cloud": "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",
        "drop": "iVBORw0KGgoAAAANSUhEUgAAAPAAAADwCAYAAAA+VemSAAAN5UlEQVR42uzde5CdZX0H8E9OVkirgAZE7qBThDKM4DiAF1p7mQJqsahIZEQBcZRa28pMYSgqioMiRSkUFBRKuFSBcBkISQhgAjUgBIFAgzQBQhIISsCES0JIyGU7nVnaBbJLsnvOnvd53+9n5p34j2fP+f2eL897fd6e3t5eEVGmVkoQkQBHRAIcEQlwRAIcEVXX0+X/ePwp9sYO2L7v3//dtsXzeBKL+v595X/f37dFNN6oLlxG2geHYxy2G+JnzMPVuAr3po2RAHd+pv8avow/avNnz8fVfdvdaWkkwO31XlyEvUfg9yzCJNyA6ViZFkcCPDRj8C38U5eOtVfgF7gBk/G7tDsS4A2zA27B7hX5nb2YhcmYgruxLu2PBPj1tsKMCoV3fX6PqZiMm7E0QyESYDbHdLyvoBqsxUzciCmYhTzhEY0L8BhMxYcLr8lTmIopuAXPZZhEEwJ8IY6pWX3W4M5+s/MDGTJRxwDvjXsbcGvmb/uF+Rd4IUMo6hDg6fjzhtVuNe7od2b7oQynKDHAH8f1KaUFmNK3TcdLKUlUPcA9eBC7pZSv8hKmY3Lf9nhKElUM8MGYmDK+odn9bvGcmZtIoioBPh9fThk3yjOYghtwE5anJNGtAC/ETinjkK3CbZjYty1KSWKkArwnZqeEbXUvru/b/ivliE4G+AScnhJ2zHxc17fdnuPmaHeAm3jtt5vHzdfjWkzDyylJDDfAj+GdKeGIex6TcA2m5npzAjzU/++zeGtK2FUvYjKuwhSsSEkS4A3RwhqMSgkrYwUmYQKmZGZOgAfztjwEX2nLMRFXYmqOmRPg13oX5qV8RXgW1+By3Jaz2QkwvA/3pHxFPg55OX6GWSlHcwO8T9ZgLt5DuAz/kTvAmhfgsViS8tXCOtyKi3FtzmQ3I8CwBGNTwlpZhgkYjztSjnoH+C7slxLW1lxchEuwOOWopuGsYfVIyldru+F0LMI1OCivo02Aozw9+CRuxDychHekLAlwlGcXfBdP4MoarAGeAEcjvQmH4TY8iGPx5pSlrJNYb8fTKWEAnsO/41wsSDmqH2BYgT9IGeM175q6Dmfh9pSjurvQcgdPrMdofAozMBOfxuiUpZoBznrHMZh9MQEP4yvZW6tegJ9ICWMDn177ERbgJGyRkpAZOEqzNb6LhTgVW6Uk3Q3wwpQwhmALfB0LcDrenpKQk1hRmjfjBMzvC/KWKUkCHGUH+ZQcI4/cdeAt8FzKGG22FKfhXKxMOToXYFiGt6SU0QFP4GRcmrW8OrMLDU+mjNEhO2I87sNfpRydCXCuBUen7YWbMSkvk29/gHMiK0bKxzAbZ2LzlCO70FHmo4zHYS4+1/S3gyTAUaptcCn+E3skwAlwlOlPcD++18SHJRLgqMtu9T/jgaYt85OTWFEnu+JWnI/NyI0cG2IUVuFNGT9RIQtxdF+gyQw8oF48lfESFbMzpuHf6nxs3GrjW+8iqmYU/h73YK8EmJzIihLtgbtxXAKcAEeZNsGZmFinl/K1K8A5Ex2lOBiz6vJivszA0UQ74Zc4NgFOgKPcXerzcDE2TYAjynQkbsM2Tb2RA/4QL2YsRMEW4WDc38QZeAWezRiIgu2AGTioiQGWM9FRA2/BJHypiQHOcXDUwWj8BN8kM3BEqb6Df636ih8JcMTAvoYL25yTygY4q1NGHX0B46sa4gQ44o19HpdWMcTZhY7YMJ/FBVU7Js4MHLFxu9Nn1jXAy/Ois2jIia1v1jHAMgtHgy4xHV3HAD+e3kZD/BQHkhk4okQ9mIDdyQwcUaLNMRFvTYAjyl1M/opuXSNudWAx7YimORAnl/xAf/+1hhLiaKJ1OADTSg7waKxET/oZDbQYe/X9W+Qu9No8FxwN9g5cOJJ/sBMH3gvSx2iwv8YxJQc4x8DRdGdhZzIDR5S6ttaPSw3w/PQvwkfx6RID/Fh6FwHOxmZkBo4o0bY4qaTrwNDCSrwp/YuwErt36uRuq0N3pORMdAQwBqd16sNbHfrceelbxP/5DN5TUoBzIivi/43Ct8kMHFGqQ7B3KQF+NP2KeN0sfGICHFGuT2HHUnahe9OviFfpwVerfh24/wJ3O6RnEa/yLLbHS1WegeGR9Cridd6GT7brwzoZ4LnpVcR6HVNCgB9OnyLW68/wrgQ4otxLSuPILnREyZeUKn0WejRWYJP0KmK9dhnugz+dnIHX5oaOiEH9zXA/oNXhLzgnPYoY0AFVD/B/p0cRA/rwcBe+6HSAH0qPIgZdvfL9ZAaOKNX+VT8GXpceRQxovyoH+KWszhExqH2rHGB4MD2KGHTp2e0T4Ihy/XGVAzw7/YkY1O4JcES5dqtygB/GyvQoYkC7VjnAa3NDR8SgKn0SC+5PjyIGtF3VA/xAehQxoLEYU+UAz0qPIt4wxJXehc460RGDP9hQ2QAvy8P9EYParMoBhnvTo4gyZ2C4Jz2KaG8WE+CIpqV+iO7LiayIAfVWPcDLskJHxKB3LFY6wHB3+hSxXstLCPDM9CliwD3Uygf4zvQpon0B7uSrVQZ63cpzQ73mFVFTvdgUq6s+A6/NcXDE6zw9lPB2I8Dwq/Qr4lWeHOr/sRsBvj39iig3wHdmsfeIV3mspAC/kAf8I163blwxAYZfpmcRCXBEHQx50ceRvg78iq3wNEald9Fwi7FNaTPw77PgewTDXeii1cUvfmt6F1FugKeldxHDez6gW8fAsBmWoic9jIZai7F4ocQZeFkeL4yGmzWc8HY7wHBzehgNdttwP6DbAb4pPYwGmzrcD+jmMTC08DS2TC+jgUvobImXhxugblqX3ehoqFuGG94qBBgmp5fRQNe340O6vQsNY/E0Rqen0RAvY2s8X4cZeGkWu4uGuakd4a1KgLVrdyKiEFe264OqsAsNuw7nmciIgryAbbGiTjPwI8N5JjKiIFe0K7xVCjBcm95GA1zUzg+rUoCvTm+j5h5o9/3/rYr9uEfT46ixc9r9ga2K/cAJ6XHU1BL8vO4BviJ9jpr6KV5q94dW5TJSf7/BHul31MhK7ILFdZ+B4Wfpd9TM+E6EV0Vn4J0xP0vORk2swbsxvxMfXsUZeGFegBY1ckmnwlvVAMPF6XvU5Kmj73TyD1Q1wBPwYvofNTjz/HgTA7wcV6X/UbDlOLXTf6RV4QJckDEQBft+p848V/0sdH+zsWfGQhTmCezWiRs3SpqB4ScZC1GgE0YivAqYgTfHk3hLxkQUYjr+cqT+WKuA1Qsuy5iIQqzG343kH2wVUJQfZVxEIc7AnJH8g1Xfhe6/it8BGR9RYXOxF1aRGfi1zsz4iArrxRdHOrwlBfhm/CbjJCq80kZX7t9vFfRfuH/JOIkKmoMTu/XHSzkGhh48ip0zZqJCjwp+APd06wu0CivWDzJmokK+1c3wKmwGhjGYh+0ydqLLpuEArOvml2gVuLbQ9zN2ossW44huh1eBMzBsinnYPuMoumAtDsS0KnyZVoEFXNXpVQ4iBvH1qoRXoTMw9OAh7JrxFCPoGhxapS/UKrSQa/CNjKcY4WfTj6ralyp1BoZRuAMfyNiKDnsG+2Bh1b5Yq+Ci9uK4jK0YgZUlP1HF8JYeYJjZiRdGRQB6cRTuqOoXLHkX+hXbYQ42y3iLNju+6nf/tWpQ5N/ilIy1aLOzqx5eNZmBoQezsoJltMllOBKVD0erJgVfgy9W4da2KN5EfKGE8NYpwDAT52T8xTDcgnFYU8oXrssu9CvejNl4Z8ZibKRpOHik1nMmM/D6vIgjsysdG+lWfLy08NYxwDAji+DFRq639jGsKPHL120Xuv8jh3fjPRmf8QYnrA7rxmqSZAYezCqMyzuGYxA/x6Elh7fOAYY5I/2aiyjGWTgCq0v/Ia2aN+oSXJTxGoBenIjjSrnO29Rj4NceD8/APhm/jbYKR+GKOv2oJgQYdsQ92DrjuJGW4JBuvT2B7EK3443pnyj9hEUMyUPYr47hbVKA4Vel3KAebb1M9H7Mq+sPbDWsoVdmLa1G6MUpOATL6vxDm3IMvL7LCP+YcV5LS3EEbmzCj21qgEdhPI7MeK+VX+MwLGjKD241tNG9OAZXZ8zXpp9n4ENNCi/0NPwVGYf3/TsuGSjWUzgKNzXxx7ca3vw1+CwuTw6KfVPCnk0NbwIMrMUROC+lKMZz+DwOxZImFyIBBtbhKzg5pai867AHLkspmnsWejBH43xsklJU7lj3H3BVSpEAv5H9cU3una7M3tGP8Q08n3IkwBtqJ1yH96YUXXMXvop7UwpyDLxxHscHc3KrK57EEfhgwktm4OE7DBdg85Sio5bhDPyw1EXmEuBqP1N8IQ5IKTryCs/zcSqeSTkS4E76En6QtyG2xWqMx/eq+v7dBLietsMP8ZmUYkhWYTxOw+MpRwLcLX+Bc7BHSrHBd1Cdh7OxOOVIgKtgNI7Et7FjyrFec3EuLqn7A/YJcLnG4G9xPLZNOazBJJyHW7KUUQJc0jK2n8PxeHfzfr6HcREuwVMZDglwyTfJfATH4qM1v2nmd7gCl+PXaX0CXDc74hgcXqNZ+TFc17fdkVe6JsBNsTfG4RDsXtD3XokZuKlvezCtTICbbhd8BAdhf4yt0Hdbhrtwe992Z4kvwE6AYyRXytwNH8K+2LNvG4n7rxdhDh7ALNyHudktToCjPY817opdsHPfthXG9m1bYNO+bRPAarzct72ApXgWS7Co3zYfD2N5ypwAR8QIy/PAEQlwRCTAEZEARyTAEVF5/zMAhk6cQUCF5KkAAAAASUVORK5CYII=",
        "sparkle": "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",
        "clover": "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",
        "flower": "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",
        "heart": "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",
        "hex": "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",
        "star": "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",
        "cushion": "iVBORw0KGgoAAAANSUhEUgAAAPAAAADwCAYAAAA+VemSAAANqUlEQVR42uzda4xc5X0/8I/HNgaMwsXYXkfw/xeEaBogtglgg1KggG/LRYEQUqhoX0RtpVR9E4moldqEJm/aqO/aqlErpEoUaGUrpQFfWHMp0GK7QGxzK6WBRILgGxc7whDfdvtmTabB3p2dmd3znJnvRzqypZ3dOfv8znd/58xzLjNGRkZERD01MgQRCXBEJMARkQBHJMARUboZGYKYou3sXJw/uvw/zMfA6L9zccLo644uB7BndNmNXdiGZ7EVH2VYmZZppOiy+VjctCzEuV1uFofxEtbhXryaAEdM3Am4GJc3LWdVsB7P4t7RZW8CHHFsJ+FyXIUrsRQnFrR+7+E7+BscSoCj383EUlyDa7EEJ9RgvV/HH2FNAhz95gIsw3Jcidk1/l0ewO9ifwIcvWoOlmEFluPTPfb7vYRb8D8JcPSC6ViClaPL5/vgfICf4Q6sTYCjjhZgJVZhGU7rwzE4iJuxLgGO0s3AFVg1uizMkICfYxBPJMBRYpdd1dRlT82QHNN+LMOmBDiqPpa9HINYhUUZkpbtxiLsSIBjqk9TXIlBLO/TY9lueQLXYTgBjsnSwJKmLnsxpmVYuubPcHcCHN00FyuauuycDMmkGcbVeDoBjk667KVYNbpckuu0p9QrWFTXc6cT4GrMwQqswkqcmSGp1F34ywQ4jmcaPt90LHtZumxRPsBn8NMEOI46A8sxiBWYlyEp2v34rQS4v7vsxViFQVyG6RmW2hjBRXg5Ae4fp2N507Hs/AxJra3GbQlwb3fZxU1ddkm6bE8ZwUK8mACTLht1tAZfToDr3WUXNXXZpemyfeUIzsNP6nLZWXAqlmEQK7EgQ9K3puMPcBfpwCW7CIOjyxX5YxZN9uKsOtxLq58CPBvX4nqswtnZTmMMX8PfJsDVOhfXjy5XY1a2y2jRC3W4k0mvBfjoRe434gZ8NtthdOASPF/yCs7okV3jlbgJg7kwILroq6UHuK4deC5uxM24rrDHe0Tv2IcFJT8JsU4BHsAt+DJ+PXOzMUXuwAMJcPvXzd6G38QXcgleVOBh3JgAt+5kfBF3YDlmZhuKCh3CAN4rceVK6miX4nvYgftwfcIbBZhZ8rnRVXfgU/A7+H1clG0lCvUkrk6Af+FX8If4ap4gEDW5e+Wnsavfd6Evwmr8CF9PeKMmGril1BWbqodGr8Z23JopoKihW/txF/osfBdfyRRQ9MB1wgN4p6SVakziJ3d34b9we8IbPWB6ifPBkxGsK7EN38UpqXv0kC/28i50A9/Cn6TjRo/6CGfiw1JWqFtBW4DH8M2EN3rYSVhZ0gp1I2xXYFupE90RXXZjL+1CX4MfYHbqGn1iDwZKeTB4Jx14EGsT3ugzc7G0lJVpN8A34F9yIX30qZvqvAt9Pp7Fp1LH6FMv48I6Bng2Npey8hEVOqeEpzdMdBf67xPeCHB9CSvRmODJ3LenbhGUMp3U6i70TLyC81K3CHAAZ1R9VlZjAo+ZSHgjfmEWrq16JVoJ8Kn409Qrorzj4FYC/NuYk1pFHPNkpuIDfGfqFHFMZ1c9KzNegH8Vl6ZOEWV24YZ034jaBni8aaStWJQaRRzXYZyJfaV14BNz1lVES4/ova6qNx8rwIt75PnBET27Gz1WgPPhVURrVpUY4Bz7RrR+T7hFpQV4IHWJKLsLN8a5dUhEJMARPe9ynFZSgOelJhEtm4FlJQV4VmoSUfZu9FhnYh3ACalJRMt24tMYKaEDH049IiZkYKqnkxLgiBrvRo8V4IOpRUR9A/xOahHR1nTS6SUEeGdqEdHWk/yXlRDgXalFRNm70enAEd23EtOqDvCbqUNE29NJi6sO8H+nDhFl70aPFeDXUoOIsgM81qmUM/FhbqsT0ZYjmIv3q+rAh/Dj1CGi7emk5ZP9Jo2xv2xb6hBR7m70eAHemhpEdBTgaQlwRD3Nw8UJcES9u3BlAd6Ft1KDiDJv+t4Y/yU2pQYRbVuCM6oM8ObUIKKjjC0nHTiC7Ea3fCZW890p9+LE1CGiLXswfzJudtdKBz6ALalBRNvm4pLJ+MGNFl/3ZGoQUd50UqsBfirjH9GR66s6BoaT8X5u9B7RtmHM7/bNIlvtwB/mODii46ytmIwf2qrHU4OIsqaTEuCIqbNigpnraoA348PUIKJtc3BZVQE+mOmkiLJ2oyfazjdm/CPKmQ9udRrpqAvxYmoQ0bYRDGB3FR34JexIDSLaNg0ru/XD2vlEbCg1iCjjOLidAD+S8Y/oyHJMryrAGyfjsqiIPnI6llQV4HfwfGoQUf1udKPN71uf8Y+ob4A3ZPwjOrIIA1UFeAv2pgYR1U4nNTp48lrOyoqoeDe60cH3Zjc6ojPLOn18bwIcUZ3TsLSqAL+NF1KDiOoubmh0+ObrMv4R1R0HdxrgzAdHdGYhFlQV4GewLzWI6Gg6aUVVAT6MR1ODiGqOgxudv3eOgyO6MJ00vaoAZzopovOrky6rKsBvY3tqENGRlVUFWHajIxLgiH52CeZUFeBNuTopoiMNXFtVgI/kZncRHVtRVYBlNzpi6gM80Ru7j2U+dmBa6hDRtgvwShUdeBd+mPGP6PikDlUEWC5uiKh3gHMcHNGZqzCzqgBvwXupQUTbTpnIaZXdDvBwHr0S0bFrqgqwHAdHTF2AuzmNdNRc7JykPw4R/eAATsdH471wMkK2J89OiujIrFbvVjlZXTKfRkd0/ml0ZQHOcXBEZ66s6hgYGtjdzuVREQE+wmk4OF7QJsNwbrUT0ZGTsGi8F03mJ8U5Do7ozOVVBngIw6lBRNuWVhngd/BsahBRzw4su9ERHfn/mFdlgDOdFNGZxVUG+DnsSQ0i6hngkUwnRdQ3wLIbHdHx40en/EysZnOwO1cnRbTlEE7G4WN9cSpC9S62pA4RbZmJc473xanqiplOimjf+VUHOMfBETUO8A+xO3WIaMvZVQd4JF04om0DVQdYAhzR0WOLKg/wRhxJLSLq2YHfy3RSRNt3eq08wDKdFNGWE0oJcI6DIyZuRpWnUv6f98OOsQ7KI+KYN3o/sYQOnKuTIrrYgRsVrEx2oyMm5mBJAR7KdFLEhOwpKcDvY3NqElHPAMt0UkS9A5zj4IjW7SwtwNvGWqmI+EReigpwrk6KaN2zpQVYAhzRksPYWmKAh453o66I+NjL+KjEAO/DM6lPxJgeHuuLjYpXLtNJEWP7xwQ4or4fXr1acoBfxFupU8Qx3TveCxrVr2O6cMRx7mBTiwCvTa0iPuHb2Dvei6b6gv5jmY13MSs1iwCv47NjXUZYUgfejydTs4iP/XEr4S0lwLIbHfGxB7C61ReXsAsN5+L11C7krKsl2N/qN5TSgd8Yb74rosf9DLdMJLwlBdh4p4xF9LCDuAOvTfQbSwrwv6aO0Yd+jpvb/RyolGNgmI5dmJOaRp/Yj5vweLs/oKQOfCSfRkcf2Y3lnYS3tADDQ6lr9IEnsLAbl9OWtAsNp2DP8R4jEVFzw/gOvj36/46V1oE/wKOpc/SgV/AbuLtb4S0xwPBgah095AN8A4vwVLd/eGm70DAPb2N6ah81Nox/xl346WS9SaPQT+eeTv2jpkawGp/DHZMZ3lIDDGuyHUTNHMEaLMRteHkq3rTEXWhYgLcK/gMTcdRe3IO/xk+m+s1LDTA8jS9k+4hCvYC/wz9M9AKEKXnydwHuT4CjMPvwAO7BcyWsUMkd+EzsKPyPTPS+QxjCfXhwrKckJMCftBaD2Yaigimgf2+6O8a7pa5o6d3t/gQ4pvBT5KewBt+vy+NvS+/AJ2MHPpXtKybpWtyNeBAPjfUkfNKB2/Eh/gm/l20tuuQdrMUP8EiVnyD3QweGy7Al2110eCHBw3gIz3TzYoIEuDUv4YJsh9GiA/g3rMXD+HGv/qJ1maL5Hv4q22WM4U2sG10eq/uuca914FPwFk7NdhqAw/iPptC+1I+DMKNG11Teg69nu+1rO7AB67AR+/p9QOrSgeEc/CgXOPTd3OwmrMc6bMdIhqWeAYY1+FLK1tN2YgPWY6iVR2wmwPXxOWzDtJSup7rsZqwfXbamy/ZugGE1bk3pam3XL3XZ9zMk/RPgC/FCunDtuuyWpmPZdNk+DjDcj9tTvqLt/qUu+16GJAE+6iy8itkpYTGG8Z9NXfb5dNkEeCzfwF+khJXag0ewDkMlXzebAJdnJrbj11LGKe2yz2Ed1o/+fzjDkgC36yo8npM7JtW7GMI6PFLHa2YT4LLdjW+llF0zgueb5mW3pMsmwJOpgcdwdcrZtvcxhPXYgF0ZkgR4qm8Evw3zUtKWjGB705U8m3Ekw5IAV+kKDGVq6bj2YSPWYQN2ZEgS4NJcg7V5QPjHtjUdy27C4QxJAly6QTyImX16LPto07FsumwCXEs34L4+uB3tMJ7HhtFlS45lE+BecT6+34M3w3sbQ3gEG3P2UwLcy7/fbNyDr9T4d9iPJ7ERQ3glm230S4CP+hL+HOfVYF0PYjMex2PYgkPZVKOfAwwz8TV8E2cU9niPzXgST2FTaU/AiwS4JKfhztHl0oruX7ypadmKg9kUIwGeuM/gTgziwi7fZvcw3sB2bG1acppiJMCT4CQsxqVYiAHMxbzRf2fhcNNyEHuwE7tG/30Tr40ub+TEiUiAI2LMK3kiIgGOiAQ4IhLgiAQ4Ior2vwMAK4lVTNh+hu0AAAAASUVORK5CYII="
    ]
}
