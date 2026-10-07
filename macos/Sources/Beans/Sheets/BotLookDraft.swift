import Foundation

/// The Look sheet's unsaved look: the base or one state's override is edited at a time, and
/// nothing reaches the account until Save.
struct BotLookDraft: Equatable {
    /// Nil edits the base appearance; a state edits its override, which inherits the rest.
    var editing: BotAvatarState?
    private(set) var look: BotLook
    let saved: BotLook?

    init(saved: BotLook?) {
        self.saved = saved
        look = saved ?? BotLook()
    }

    /// What Save sends: nil when the draft is the seeded default with no overrides, so the bot
    /// goes back to its generated default instead of storing an empty look.
    var result: BotLook? { Self.canonical(look) }

    var hasChanges: Bool { result != Self.canonical(saved) }

    /// The fields set on what is being edited, without inheritance.
    var own: BotAppearance {
        guard let editing else { return look.base }
        return look.states[editing] ?? BotAppearance()
    }

    /// What is being edited shows: its own fields over the base.
    var shown: BotAppearance {
        editing.map { look.resolved(for: $0) } ?? look.base
    }

    /// Whether a state's override sets anything of its own.
    func overrides(_ state: BotAvatarState) -> Bool {
        !(look.states[state]?.isEmpty ?? true)
    }

    mutating func update(_ change: (inout BotAppearance) -> Void) {
        guard let editing else {
            change(&look.base)
            return
        }
        var appearance = look.states[editing] ?? BotAppearance()
        change(&appearance)
        look.states[editing] = appearance.isEmpty ? nil : appearance
    }

    /// The edited state inherits everything from the base again.
    mutating func resetState() {
        guard let editing else { return }
        look.states[editing] = nil
    }

    /// Back to the bot-ID seeded default: no base fields and no overrides. The photo is separate.
    mutating func resetAll() {
        look = BotLook()
    }

    /// A random shape, expression, background, hue, and tone for what is being edited. Explicit
    /// colors are cleared so the new hue shows; motion is left as it was.
    mutating func shuffle<G: RandomNumberGenerator>(using generator: inout G) {
        let shape = BotAppearance.Shape.allCases.randomElement(using: &generator)
        let expression = BotAppearance.Expression.allCases.randomElement(using: &generator)
        let background = BotAppearance.Background.allCases.randomElement(using: &generator)
        let tone = BotAppearance.Tone.allCases.randomElement(using: &generator)
        let hue = Double(Int.random(in: 0..<360, using: &generator))
        update {
            $0.shape = shape
            $0.expression = expression
            $0.background = background
            $0.tone = tone
            $0.hue = hue
            $0.palette = .init()
        }
    }

    /// Drops only empty overrides. An explicit choice equal to today's base still expresses
    /// intent: it must survive a later base edit.
    static func canonical(_ look: BotLook?) -> BotLook? {
        guard var copy = look else { return nil }
        copy.states = copy.states.filter { !$0.value.isEmpty }
        return copy.base.isEmpty && copy.states.isEmpty ? nil : copy
    }
}
