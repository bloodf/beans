// Run with macos/Tests/run-avatar-model.sh; exercises the look schema, activity precedence, and
// the Look sheet's draft without launching the app.
import Foundation

@main
struct AvatarModelChecks {
    static func main() throws {
        var failures: [String] = []
        func check(_ condition: Bool, _ message: String, line: Int = #line) {
            if !condition { failures.append("line \(line): \(message)") }
        }
        let decoder = JSONDecoder()
        func decode(_ json: String) -> BotLook? { try? decoder.decode(BotLook.Lenient.self, from: Data(json.utf8)).value }
        func roundTrip(_ look: BotLook) -> BotLook? {
            let data = try! JSONSerialization.data(withJSONObject: look.wireObject)
            return try? decoder.decode(BotLook.self, from: data)
        }

        // Schema: the contract's enums, exactly.
        check(BotAvatarState.allCases.map(\.rawValue) == ["idle", "thinking", "responding", "working", "waiting", "retry", "error"], "states")
        check(BotAppearance.Shape.allCases.map(\.rawValue) == ["round", "organic", "boxy", "capsule", "nub", "cloud", "droplet", "hexagon", "sun", "triangle"], "shapes")
        check(BotAppearance.Expression.allCases.count == 14 && BotAppearance.Expression.allCases.first == .idle, "expressions")
        check(BotAppearance.Background.allCases.map(\.rawValue) == ["none", "square", "circle", "squircle"], "backgrounds")
        check(BotAppearance.Tone.allCases.map(\.rawValue) == ["pastel", "pale", "mid", "deep", "bright", "ink"], "tones")

        // Decoding: the core's full fixture round-trips; sparse looks keep nil for seeded fields.
        let full = decode(##"{"version":1,"base":{"shape":"cloud","expression":"happy","background":"square","hue":359.5,"tone":"ink","palette":{"head":"#ABCDEF","eye":"#010203","bg":"#FFFFFF"},"motion":false},"states":{"working":{"shape":"boxy","palette":{"eye":"#102030"}},"error":{"expression":"sad"}}}"##)
        check(full?.base.shape == .cloud && full?.base.hue == 359.5 && full?.base.motion == false, "full base")
        check(full?.states[.working]?.palette.eye == "#102030" && full?.states[.error]?.expression == .sad, "full states")
        check(full.flatMap(roundTrip) == full, "wire round trip")
        let sparse = decode(#"{"version":1,"base":{}}"#)
        check(sparse == BotLook(), "sparse base")
        check(roundTrip(BotLook())?.base.isEmpty == true, "empty wire base")
        check(BotLook().wireObject["states"] == nil, "no empty states object")

        // Lenient reading: unreadable envelopes keep the seed; unknown values fall back per field.
        for bad in [#"{"version":2,"base":{}}"#, #"{"version":1}"#, #"false"#, #"{"version":1,"base":null}"#] {
            check(decode(bad) == nil, "envelope \(bad)")
        }
        let future = decode(##"{"version":1,"base":{"shape":"star","hue":360,"tone":"neon","palette":{"head":"#abcdef","eye":"#010203","future":"#000000"},"future_trait":1},"states":{"dancing":{"shape":"round"},"thinking":{"expression":"thinking"}}}"##)
        check(future?.base.shape == nil && future?.base.hue == nil && future?.base.tone == nil, "unknown values fall back")
        check(future?.base.palette == .init(head: nil, eye: "#010203", bg: nil), "non-canonical color ignored")
        check(future?.states.keys.sorted { $0.rawValue < $1.rawValue } == [.thinking], "unknown state skipped")

        // Inheritance: omitted fields and palette channels come from the base.
        var look = BotLook()
        look.base = BotAppearance(shape: .round, expression: .happy, hue: 10, palette: .init(head: "#112233", eye: "#445566"), motion: false)
        look.states[.working] = BotAppearance(shape: .boxy, palette: .init(eye: "#778899"))
        let working = look.resolved(for: .working)
        check(working.shape == .boxy && working.expression == .happy && working.hue == 10 && working.motion == false, "field inheritance")
        check(working.palette == .init(head: "#112233", eye: "#778899", bg: nil), "palette deep merge")
        check(look.resolved(for: .idle) == look.base, "no override is the base")

        // Validation helpers.
        check(BotLook.canonicalHex("abcdef") == "#ABCDEF" && BotLook.canonicalHex(" #a1B2c3 ") == "#A1B2C3", "hex canonical")
        for bad in ["#fff", "#GGGGGG", "rgb(1,2,3)", "", "#1234567"] { check(BotLook.canonicalHex(bad) == nil, "hex \(bad)") }
        check(BotAppearance.normalizedHue(360) == 0 && BotAppearance.normalizedHue(-30) == 330 && BotAppearance.normalizedHue(.nan) == nil, "hue wrap")
        check(BotAppearance(hue: 360).isValid == false && BotAppearance(palette: .init(bg: "#abcdef")).isValid == false, "invalid appearance")
        check(abs((BotLook.contrastRatio("#000000", "#FFFFFF") ?? 0) - 21) < 0.01 && BotLook.contrastRatio("#777777", "#777777") == 1, "contrast")

        // Activity precedence: strongest signal wins, generic work stays idle.
        check(BotActivity.state(.init()) == .idle, "nothing is idle")
        check(BotActivity.state(.init(hasError: true, isRetrying: true, isWaiting: true, isRunningTool: true, isStreaming: true, isThinking: true)) == .error, "error first")
        check(BotActivity.state(.init(isRetrying: true, isWaiting: true)) == .retry, "retry over waiting")
        check(BotActivity.state(.init(isWaiting: true, isRunningTool: true)) == .waiting, "waiting over tool")
        check(BotActivity.state(.init(isRunningTool: true, isStreaming: true)) == .working, "tool over streaming")
        check(BotActivity.state(.init(isStreaming: true, isThinking: true)) == .responding, "streaming over thinking")
        check(BotActivity.aggregate([.thinking, .waiting, .idle]) == .waiting && BotActivity.aggregate([]) == .idle, "aggregate")
        check(BotActivity.aggregate([.idle, .responding]) == BotActivity.aggregate([.responding, .idle]), "aggregate order-free")

        // Signals from messages.
        let now = Date()
        func tool(_ running: Bool, run: CommandRun? = nil) -> Message {
            Message(author: .bot("b"), body: .tool(ToolInvocation(name: "bash", summary: "", detail: "", isRunning: running, run: run)))
        }
        func signals(_ messages: [Message], running: Bool = true, thinking: Bool = false, retrying: Bool = false, finished: Date? = nil) -> BotAvatarState {
            BotActivity.state(BotActivity.signals(of: "b", in: messages, isRunning: running, isThinking: thinking, isRetrying: retrying, finishedAt: finished, now: now))
        }
        check(signals([]) == .idle, "running without phase is idle")
        check(signals([tool(true)]) == .working, "running tool")
        check(signals([tool(false)]) == .idle, "finished tool does not pin")
        check(signals([Message(author: .bot("b"), body: .text("hi"), state: .streaming)]) == .responding, "streaming")
        check(signals([Message(author: .bot("b"), body: .text(""), state: .thinking)]) == .thinking, "thinking message")
        check(signals([], thinking: true) == .thinking && signals([], running: false, thinking: true) == .idle, "job.thinking only while running")
        check(signals([], retrying: true) == .retry && signals([], running: false, retrying: true) == .idle, "retry only while running")
        let asking = CommandRun(command: "sudo x", state: .asking)
        check(signals([tool(true, run: asking)]) == .waiting, "command asking")
        let pending = PermissionRequest(pluginID: "p", pluginName: "P", tool: "t", summary: "", decision: .pending)
        var answered = pending
        answered.decision = .allowed
        check(signals([Message(author: .bot("b"), body: .permission(pending))]) == .waiting, "pending permission")
        check(signals([Message(author: .bot("b"), body: .permission(answered))]) == .idle, "answered permission")
        let failed = Message(author: .bot("b"), body: .text("x"), state: .failed("boom"))
        check(signals([failed], running: false, finished: now.addingTimeInterval(-1)) == .error, "fresh error")
        check(signals([failed], running: false, finished: now.addingTimeInterval(-BotActivity.errorDuration - 1)) == .idle, "error expires")
        check(signals([failed], running: false, finished: nil) == .idle, "historical error does not pin")
        check(signals([failed]) == .idle, "next turn clears error")
        check(signals([Message(author: .bot("other"), body: .text("x"), state: .streaming)]) == .idle, "other bot ignored")

        // Draft: edits stay local, overrides inherit, Save sends canonical looks.
        var draft = BotLookDraft(saved: nil)
        check(!draft.hasChanges && draft.result == nil, "fresh draft")
        draft.update { $0.shape = .sun }
        check(draft.hasChanges && draft.result?.base.shape == .sun, "base edit")
        draft.editing = .error
        check(draft.shown.shape == .sun && draft.own.isEmpty, "state shows inherited")
        draft.update { $0.expression = .sad }
        check(draft.overrides(.error) && draft.result?.states[.error]?.expression == .sad, "state override")
        draft.update { $0.shape = .sun }
        draft.update { $0.expression = nil }
        check(draft.result?.states[.error]?.shape == .sun, "explicit equal-base override is preserved")
        draft.editing = nil
        draft.update { $0.shape = .cloud }
        draft.editing = .error
        check(draft.shown.shape == .sun, "explicit state choice survives future base edits")
        draft.update { $0.expression = .mad }
        draft.resetState()
        check(!draft.overrides(.error), "reset state inherits")
        draft.editing = nil
        draft.resetAll()
        check(draft.result == nil && !draft.hasChanges, "reset all is the seeded default")
        var saved = BotLook()
        saved.base.tone = .ink
        var edit = BotLookDraft(saved: saved)
        check(!edit.hasChanges, "saved look unchanged")
        edit.update { $0.tone = .ink }
        check(!edit.hasChanges, "same value is no change")
        var generator = SplitMix(seed: 7)
        edit.update { $0.palette = .init(head: "#000000") }
        edit.shuffle(using: &generator)
        check(edit.own.shape != nil && edit.own.palette.isEmpty && edit.own.hue.map(BotAppearance.isValidHue) == true, "shuffle")
        check(edit.result?.isValid == true, "shuffled look valid")
        check(edit.saved == saved, "draft never touches the saved look")

        // Save boundaries: omitted, explicit null, and look plus photo share one payload.
        check(BotLookChange.updateParams("b", look: .keep, photo: .keep) == nil, "unchanged save sends nothing")
        let photoOnly = BotLookChange.updateParams("b", look: .keep, photo: .remove)!
        check(photoOnly["look"] == nil && photoOnly["avatar"] is NSNull, "photo-only save preserves look")
        let reset = BotLookChange.updateParams("b", look: .reset, photo: .keep)!
        check(reset["look"] is NSNull && reset["avatar"] == nil, "reset preserves photo")
        let combined = BotLookChange.updateParams("b", look: .replace(saved), photo: .set(URL(fileURLWithPath: "/tmp/photo.png")))!
        let combinedLook = try decoder.decode(BotLook.self, from: JSONSerialization.data(withJSONObject: combined["look"]!))
        check(combinedLook == saved && (combined["avatar"] as? [String: String])?["path"] == "/tmp/photo.png", "combined look/photo update")

        let user = Message(author: .you, body: .text("next"))
        let oldStream = Message(author: .bot("b"), body: .text("old"), state: .streaming)
        check(signals([oldStream, user]) == .idle, "new user clears old streaming phase")
        check(signals([Message(author: .bot("b"), body: .permission(pending)), user]) == .idle, "new user clears old pending card")
        check(signals([Message(author: .bot("b"), body: .permission(pending))], running: false) == .idle, "finished turn does not retain waiting")
        var ledger = BotActivityLedger()
        ledger.retry("b", in: "group")
        ledger.retry("other", in: "group")
        ledger.settleRetry("other", in: "group")
        check(ledger.isRetrying("b", in: "group") && !ledger.isRetrying("other", in: "group"), "group retry clears only its bot")
        ledger.turnEnded("b", in: "group", at: now)
        ledger.turnStarted("b", in: "group")
        check(ledger.finishedAt("b", in: "group") == nil, "new turn clears old error")
        ledger.retry("b", in: "group")
        ledger.remove(chatID: "group")
        check(!ledger.isRetrying("b", in: "group"), "removed chat clears retry")

        if failures.isEmpty {
            print("avatar model: ok")
        } else {
            failures.forEach { print("FAIL \($0)") }
            exit(1)
        }
    }
}

struct SplitMix: RandomNumberGenerator {
    var state: UInt64
    init(seed: UInt64) { state = seed }
    mutating func next() -> UInt64 {
        state &+= 0x9E37_79B9_7F4A_7C15
        var z = state
        z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
        z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
        return z ^ (z >> 31)
    }
}
