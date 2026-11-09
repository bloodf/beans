import AppKit

private enum OperationFailure: Error { case unavailable, stale, invalid }

/// Uses the existing protocol-5 client. Unknown methods and unsupported Runner capabilities
/// fail through the CLI; an error never substitutes cached data for an authoritative reply.
class RunnerOperationForm: MemoryFormViewController {
    let store = AppStore.shared
    private var epoch = UUID()
    private var invalidated = false
    override func loadView() {
        super.loadView()
        store.observe(self) { [weak self] event in
            switch event {
            case .connectionChanged, .identityChanged, .snapshotReplaced:
                self?.epoch = UUID(); self?.invalidated = true
                self?.errorLabel.stringValue = L("Account or connection changed. Close and reopen this screen.")
            default: break
            }
        }
        setButtons(confirm: L("Done"), cancel: nil)
    }
    override func confirmTapped() { dismissSheet() }
    override func showError(_ error: Error) {
        switch error {
        case OperationFailure.invalid:
            errorLabel.stringValue = L("Enter the required IDs and valid nonnegative numbers within the displayed limits. Your draft is preserved.")
        case OperationFailure.stale:
            errorLabel.stringValue = L("The account or connection changed. Close and reopen this screen before acting.")
        case OperationFailure.unavailable:
            errorLabel.stringValue = L("Wait for an authenticated account snapshot, then reopen this screen.")
        default:
            errorLabel.stringValue = L("Operation failed. Check the account, Runner connection and CLI version/capability. Reopen this screen after reconnecting. Your draft is preserved.")
        }
    }
    func call<T: Decodable>(_ method: String, _ params: [String: Any], as type: T.Type) async throws -> T {
        guard !invalidated, store.isConnected, store.hasIdentity == true,
              let owner = store.deviceNameOwner else { throw OperationFailure.unavailable }
        let token = epoch
        let result = try await store.client.request(method, params, as: type)
        guard !invalidated, epoch == token, store.deviceNameOwner == owner else { throw OperationFailure.stale }
        return result
    }
    func action(_ title: String, _ work: @escaping @MainActor () async throws -> Void) {
        addForm(MemoryActionButton(title) { [weak self] in self?.run(work) })
    }
    func required(_ field: NSTextField) throws -> String {
        let value = field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !value.isEmpty else { throw OperationFailure.invalid }; return value
    }
}

final class RunnerOperationsViewController: RunnerOperationForm {
    init() { super.init(title: L("Runner operations"), subtitle: L("Changes run on the selected bot's Runner. The CLI enforces account authority and supported capabilities."), width: 580) }
    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }
    override func loadView() {
        super.loadView()
        note("Budgets replace all limits for a scope. Connector limits can end a service cooldown. Browser deletion removes a profile. Review each action before confirming.")
        action("Budgets…") { self.presentAsSheet(OperationBudgetViewController()) }
        action("Connector limits…") { self.presentAsSheet(OperationConnectorViewController()) }
        action("Browser profiles…") { self.presentAsSheet(OperationBrowserViewController()) }
        action("Feedback…") { self.presentAsSheet(OperationFeedbackViewController()) }
        action("Channels…") { self.presentAsSheet(OperationChannelsViewController()) }
    }
}

private struct OperationEmpty: Decodable {}
private struct OperationBudget: Decodable { let kind: String; let id: String; let state: String }

final class OperationBudgetViewController: RunnerOperationForm {
    private var runner: NSTextField!, bot: NSTextField!, identifier: NSTextField!, kind: NSPopUpButton!
    private var usd: NSTextField!, tokens: NSTextField!, seconds: NSTextField!, retries: NSTextField!, calls: NSTextField!
    private let summary = NSTextField(wrappingLabelWithString: "")
    init() { super.init(title: L("Budgets"), subtitle: L("Replace limits for a chat, task, routine or existing job."), width: 580) }
    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }
    override func loadView() {
        super.loadView()
        runner = text("Runner ID"); bot = text("Bot ID")
        kind = popup("Scope", choices: ["chat", "task", "routine", "job"]); identifier = text("Scope ID")
        usd = text("Maximum USD"); tokens = text("Maximum tokens"); seconds = text("Maximum runtime seconds, at most 31536000")
        retries = text("Maximum retries"); calls = text("Maximum connector calls")
        note("Empty fields remove that limit. This form does not load current limits. Saving replaces the complete limits object and never resumes stopped work.")
        addForm(summary)
        action("Replace limits…") { [self] in
            var limits: [String: Any] = [:]
            for (name, field) in [("max_tokens", tokens!), ("max_runtime_secs", seconds!), ("max_retries", retries!), ("max_connector_calls", calls!)] {
                let value = field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
                if !value.isEmpty {
                    guard let number = UInt64(value), name != "max_runtime_secs" || number <= 31_536_000 else { throw OperationFailure.invalid }
                    limits[name] = number
                }
            }
            let amount = usd.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
            if !amount.isEmpty {
                guard let number = Double(amount), number.isFinite, number >= 0 else { throw OperationFailure.invalid }
                limits["max_usd"] = number
            }
            let scope = ["chat", "task", "routine", "job"][kind.indexOfSelectedItem]
            let params: [String: Any] = ["runner_id": try required(runner), "bot_id": try required(bot), "kind": scope, "id": try required(identifier), "limits": limits]
            guard await confirm("Replace all limits?", details: L("Empty fields remove limits. This affects the identified scope on the specified Runner.")) else { return }
            let result = try await call("budgets.set", params, as: OperationBudget.self)
            summary.stringValue = "\(result.kind) · \(result.id) · \(result.state)"
        }
    }
}

private struct OperationConnector: Decodable {
    struct Limits: Decodable { let maxCalls: UInt32; let windowSecs: UInt64; let maxConcurrency: UInt32 }
    let pluginId: String; let serviceId: String; let limits: Limits; let retryAt: Double?
}
final class OperationConnectorViewController: RunnerOperationForm {
    private var runner: NSTextField!, plugin: NSTextField!, scope: NSPopUpButton!
    private var calls: NSTextField!, seconds: NSTextField!, concurrency: NSTextField!
    private let summary = NSTextField(wrappingLabelWithString: "")
    init() { super.init(title: L("Connector limits"), subtitle: L("Account or service call admission on one Runner."), width: 580) }
    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }
    private func params() throws -> [String: Any] {
        ["runner_id": try required(runner), "plugin_id": try required(plugin), "scope": scope.indexOfSelectedItem == 0 ? "account" : "service"]
    }
    private func display(_ result: OperationConnector) {
        calls.stringValue = String(result.limits.maxCalls); seconds.stringValue = String(result.limits.windowSecs)
        concurrency.stringValue = String(result.limits.maxConcurrency)
        summary.stringValue = "\(result.pluginId) · \(result.serviceId)" + (result.retryAt == nil ? "" : " · " + L("Service cooldown active"))
    }
    override func loadView() {
        super.loadView(); runner = text("Runner ID"); plugin = text("Installed plugin/account ID")
        scope = popup("Scope", choices: ["account", "service"])
        calls = text("Calls per window, 0–10000", value: "60"); seconds = text("Window seconds, 1–86400", value: "60"); concurrency = text("Maximum concurrency, 0–256", value: "4")
        addForm(summary)
        action("Load limits") { [self] in display(try await call("connector_limits.get", try params(), as: OperationConnector.self)) }
        action("Save limits…") { [self] in
            guard let count = UInt32(calls.stringValue), count <= 10_000,
                  let window = UInt64(seconds.stringValue), (1...86_400).contains(window),
                  let parallel = UInt32(concurrency.stringValue), parallel <= 256 else { throw OperationFailure.invalid }
            var body = try params(); body["limits"] = ["max_calls": UInt64(count), "window_secs": window, "max_concurrency": UInt64(parallel)]
            guard await confirm("Save connector limits?", details: L("This also ends the installed account's service cooldown. Service scope affects accounts sharing that service.")) else { return }
            display(try await call("connector_limits.set", body, as: OperationConnector.self))
        }
    }
}

private struct OperationSessions: Decodable {
    struct Session: Decodable { let id: String; let name: String; let state: String; let revision: UInt64 }
    let sessions: [Session]
}
final class OperationBrowserViewController: RunnerOperationForm {
    private var bot: NSTextField!, name: NSTextField!
    private let menu = NSPopUpButton()
    private var sessions: [OperationSessions.Session] = []
    init() { super.init(title: L("Browser profiles"), subtitle: L("Profiles belong to a bot's Runner. Deletion removes stored browser state."), width: 580) }
    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }
    private var loadedBot: String?
    private func reload() async throws {
        let id = try required(bot)
        let result = try await call("browser.sessions", ["bot_id": id], as: OperationSessions.self)
        sessions = result.sessions; loadedBot = id; menu.removeAllItems()
        for session in sessions { menu.addItem(withTitle: "\(session.name) · \(session.state) · \(session.id)") }
    }
    override func loadView() {
        super.loadView(); bot = text("Bot ID"); name = text("New profile name"); row("Profile", menu)
        action("Refresh profiles") { [self] in try await reload() }
        action("Create profile") { [self] in
            let _: OperationEmpty = try await call("browser.create", ["bot_id": try required(bot), "name": name.stringValue], as: OperationEmpty.self)
            try await reload()
        }
        for (title, method) in [("Stop profile…", "browser.stop"), ("Delete profile…", "browser.delete")] {
            action(title) { [self] in
                let id = try required(bot); let index = menu.indexOfSelectedItem
                guard loadedBot == id, sessions.indices.contains(index) else { throw OperationFailure.stale }
                let session = sessions[index]
                guard await confirm(title, details: session.name + " · " + session.id) else { return }
                let _: OperationEmpty = try await call(method, ["bot_id": id, "session_id": session.id], as: OperationEmpty.self)
                try await reload()
            }
        }
    }
}

private struct OperationFeedback: Decodable {
    struct Entry: Decodable { let id: String; let kind: String; let note: String; let example: String }
    struct Settings: Decodable { let reviewEverySecs: Int64? }
    let feedback: [Entry]; let feedbackCount: Int; let settings: Settings
}
final class OperationFeedbackViewController: RunnerOperationForm {
    private var bot: NSTextField!, days: NSTextField!
    private let summary = NSTextField(wrappingLabelWithString: "")
    init() { super.init(title: L("Feedback"), subtitle: L("Read recent included feedback and configure automatic review. No proposal is accepted here."), width: 620) }
    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }
    override func loadView() {
        super.loadView(); bot = text("Bot ID"); days = text("Review interval in days, 1–30; empty disables")
        addForm(summary)
        action("Load feedback") { [self] in
            let result = try await call("feedback.list", ["bot_id": try required(bot)], as: OperationFeedback.self)
            days.stringValue = result.settings.reviewEverySecs.map { String($0 / 86_400) } ?? ""
            summary.stringValue = String(result.feedbackCount) + " " + L("included feedback records")
            output(result.feedback.map { "\($0.kind) · \($0.id)\n\($0.note)\n\($0.example)" }.joined(separator: "\n\n"))
        }
        action("Save review interval…") { [self] in
            let value = days.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
            var interval: Any = NSNull()
            if !value.isEmpty { guard let count = Int64(value), (1...30).contains(count) else { throw OperationFailure.invalid }; interval = count * 86_400 }
            let id = try required(bot)
            guard await confirm("Change automatic review?", details: L("Automatic reviews can use the bot's model and incur costs. An empty interval disables automatic review.")) else { return }
            let _: OperationEmpty = try await call("feedback.settings", ["bot_id": id, "review_every_secs": interval], as: OperationEmpty.self)
            summary.stringValue = L("Review interval saved.")
        }
    }
}

private struct OperationSubscriptions: Decodable {
    struct Subscription: Decodable {
        struct Config: Decodable {
            struct Channel: Decodable { let accountId: String }
            let name: String; let botId: String; let source: String; let channel: Channel?
        }
        let id: String; let config: Config; let state: String
    }
    let subscriptions: [Subscription]
}
final class OperationChannelsViewController: RunnerOperationForm {
    private var runner: NSTextField!
    private let menu = NSPopUpButton()
    private var channels: [OperationSubscriptions.Subscription] = []
    private var loadedRunner: String?
    init() { super.init(title: L("Channels"), subtitle: L("Pause or resume existing Slack and Telegram channels. Resume can admit queued messages."), width: 580) }
    @available(*, unavailable) required init?(coder: NSCoder) { fatalError() }
    private func reload() async throws {
        let id = try required(runner)
        let result = try await call("events.list", ["runner_id": id], as: OperationSubscriptions.self)
        channels = result.subscriptions.filter { $0.config.channel != nil && ["telegram", "slack"].contains($0.config.source) }
        loadedRunner = id; menu.removeAllItems()
        for channel in channels { menu.addItem(withTitle: "\(channel.config.name) · \(channel.state) · \(channel.id)") }
    }
    override func loadView() {
        super.loadView(); runner = text("Runner ID"); row("Channel", menu)
        action("Refresh channels") { [self] in try await reload() }
        for (title, method) in [("Pause channel…", "events.pause"), ("Resume channel…", "events.resume")] {
            action(title) { [self] in
                let id = try required(runner); let index = menu.indexOfSelectedItem
                guard loadedRunner == id, channels.indices.contains(index) else { throw OperationFailure.stale }
                let channel = channels[index]
                guard await confirm(title, details: "\(channel.config.name) · \(channel.id)") else { return }
                let _: OperationEmpty = try await call(method, ["runner_id": id, "id": channel.id], as: OperationEmpty.self)
                try await reload()
            }
        }
    }
}
