import Foundation

/// What a bot is doing in one chat, from the structured job and message signals the CLI sends;
/// never from localized status text or what the model said.
struct BotActivitySignals: Hashable {
    /// The bot's turn ended in an error, shown until its next turn or `BotActivity.errorDuration`.
    var hasError = false
    /// A model call is waiting to be asked again (`job.retry`).
    var isRetrying = false
    /// A permission card or a command at a question waits on the user.
    var isWaiting = false
    /// A tool call of the bot's is running.
    var isRunningTool = false
    /// The bot's reply is streaming.
    var isStreaming = false
    /// The model is reasoning (`job.thinking`, or a message in its thinking state).
    var isThinking = false
}

enum BotActivity {
    /// How long a turn's error keeps the error look when no new turn starts.
    static let errorDuration: TimeInterval = 8

    /// Strongest first: an error, a retry, waiting on the user, a running tool, streaming text,
    /// thinking. A turn with none of these keeps the idle look; the presence dot still shows it.
    static let precedence: [BotAvatarState] = [.error, .retry, .waiting, .working, .responding, .thinking, .idle]

    static func state(_ signals: BotActivitySignals) -> BotAvatarState {
        if signals.hasError { return .error }
        if signals.isRetrying { return .retry }
        if signals.isWaiting { return .waiting }
        if signals.isRunningTool { return .working }
        if signals.isStreaming { return .responding }
        if signals.isThinking { return .thinking }
        return .idle
    }

    /// One bot across its chats, for surfaces without a chat (sidebar, inspector, lists): the
    /// strongest state, so the result does not depend on chat order.
    static func aggregate<S: Sequence>(_ states: S) -> BotAvatarState where S.Element == BotAvatarState {
        states.min { rank($0) < rank($1) } ?? .idle
    }
    private static func rank(_ state: BotAvatarState) -> Int {
        precedence.firstIndex(of: state) ?? precedence.count
    }

    /// How far back the transcript is read for the bot's live rows; anything older has settled.
    static let recentMessages = 40

    /// The bot's signals in one chat from the store's job state and the current turn: its
    /// messages since the user's last one. Cards, questions, tools, and streaming count only
    /// while a turn of the bot's runs; an error counts only while none runs, for
    /// `errorDuration` after the failed turn ended (`finishedAt`).
    static func signals(
        of botID: Bot.ID, in messages: [Message], isRunning: Bool, isThinking: Bool, isRetrying: Bool,
        finishedAt: Date?, now: Date
    ) -> BotActivitySignals {
        var signals = BotActivitySignals(isRetrying: isRunning && isRetrying, isThinking: isRunning && isThinking)
        // ponytail: scans the newest `recentMessages` rows per query; index per chat if live rows sit further back.
        let window = messages.suffix(recentMessages)
        let turn = window.lastIndex(where: { $0.author == .you }).map { window[window.index(after: $0)...] } ?? window
        let recent = turn.filter { $0.author == .bot(botID) }
        guard let last = recent.last else { return signals }
        if isRunning {
            for message in recent {
                switch message.body {
                case let .permission(request) where request.isPending: signals.isWaiting = true
                case let .tool(tool) where tool.isRunning && (tool.run?.state == .asking || tool.run?.state == .waiting):
                    signals.isWaiting = true
                default: break
                }
            }
            if case let .tool(tool) = last.body, tool.isRunning { signals.isRunningTool = true }
            if case .text = last.body, last.state == .streaming { signals.isStreaming = true }
            if last.state == .thinking { signals.isThinking = true }
        } else if case .failed = last.state, let finishedAt, now.timeIntervalSince(finishedAt) < errorDuration {
            signals.hasError = true
        }
        return signals
    }
}

/// The per-bot activity the store keeps beside its running jobs: which bot's model call waits to
/// be retried in a chat, and when each bot's last turn there ended. Keyed by chat and bot, so one
/// group member's retry or error never shows on another.
struct BotActivityLedger {
    struct Key: Hashable {
        let chatID: Chat.ID
        let botID: Bot.ID
    }

    private(set) var retrying: Set<Key> = []
    private(set) var turnEnds: [Key: Date] = [:]

    func isRetrying(_ botID: Bot.ID, in chatID: Chat.ID) -> Bool { retrying.contains(Key(chatID: chatID, botID: botID)) }
    func finishedAt(_ botID: Bot.ID, in chatID: Chat.ID) -> Date? { turnEnds[Key(chatID: chatID, botID: botID)] }

    mutating func retry(_ botID: Bot.ID, in chatID: Chat.ID) {
        retrying.insert(Key(chatID: chatID, botID: botID))
    }

    /// A message from the bot, or its turn ending, settles its retry. An empty bot (a group
    /// between member turns) settles every retry in the chat.
    mutating func settleRetry(_ botID: Bot.ID, in chatID: Chat.ID) {
        retrying = retrying.filter { $0.chatID != chatID || (!botID.isEmpty && $0.botID != botID) }
    }

    /// A new turn starts clean: no old error and no old retry.
    mutating func turnStarted(_ botID: Bot.ID, in chatID: Chat.ID) {
        let key = Key(chatID: chatID, botID: botID)
        turnEnds[key] = nil
        retrying.remove(key)
    }

    /// Returns the key whose error look now runs until `errorDuration` after `date`.
    @discardableResult
    mutating func turnEnded(_ botID: Bot.ID, in chatID: Chat.ID, at date: Date) -> Key {
        let key = Key(chatID: chatID, botID: botID)
        settleRetry(botID, in: chatID)
        turnEnds = turnEnds.filter { date.timeIntervalSince($0.value) < BotActivity.errorDuration }
        turnEnds[key] = date
        return key
    }

    mutating func remove(chatID: Chat.ID) {
        retrying = retrying.filter { $0.chatID != chatID }
        turnEnds = turnEnds.filter { $0.key.chatID != chatID }
    }

    mutating func remove(botID: Bot.ID) {
        retrying = retrying.filter { $0.botID != botID }
        turnEnds = turnEnds.filter { $0.key.botID != botID }
    }

    mutating func reset() {
        retrying.removeAll()
        turnEnds.removeAll()
    }
}
