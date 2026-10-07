import Foundation

/// What a Look save does to the saved look. `keep` leaves `look` out of the request.
enum BotLookChange: Equatable {
    case keep
    /// Back to the bot-ID seeded default (`look: null`).
    case reset
    /// The complete look replaces the saved one.
    case replace(BotLook)

    /// A draft's result against what is saved: `keep` when nothing changed.
    init(result: BotLook?, hasChanges: Bool) {
        if !hasChanges {
            self = .keep
        } else if let result {
            self = .replace(result)
        } else {
            self = .reset
        }
    }

    /// The parameters of the one `bots.update` a Look save sends, or nil when nothing changes.
    /// Each field is present only when it changes, so a save never overwrites what the sheet left
    /// alone.
    static func updateParams(_ id: String, look: BotLookChange, photo: BotPhotoChange) -> [String: Any]? {
        var params: [String: Any] = ["id": id]
        switch look {
        case .keep: break
        case .reset: params["look"] = NSNull()
        case let .replace(value): params["look"] = value.wireObject
        }
        switch photo {
        case .keep: break
        case .remove: params["avatar"] = NSNull()
        case let .set(url): params["avatar"] = ["path": url.path, "name": url.lastPathComponent, "mime": "image/png"]
        }
        return params.count > 1 ? params : nil
    }
}

/// What a Look save does to the profile image.
enum BotPhotoChange: Equatable {
    case keep
    case remove
    case set(URL)
}

/// What a bot's generated portrait shows for: the turn phase the app resolves from structured job
/// and message signals (`BotActivity`), never from localized status text or model output.
enum BotAvatarState: String, CaseIterable, Hashable {
    case idle, thinking, responding, working, waiting, retry, error
}

/// One generated appearance, as the account stores it. Every field is optional: a missing shape,
/// hue, or tone keeps the bot-ID seeded value, a missing expression is idle, a missing background
/// is none, and missing motion is on. Only these fields are authorable; there are no free-form
/// traits, functions, CSS, or SVG.
struct BotAppearance: Hashable {
    enum Shape: String, CaseIterable {
        case round, organic, boxy, capsule, nub, cloud, droplet, hexagon, sun, triangle
    }

    enum Expression: String, CaseIterable {
        case idle, happy, sad, mad, surprised, wink, sleepy, smug, unsure, scared, love, shy, sick, thinking
    }

    enum Background: String, CaseIterable {
        case none, square, circle, squircle
    }

    enum Tone: String, CaseIterable {
        case pastel, pale, mid, deep, bright, ink
    }

    /// Explicit colors, canonical `#RRGGBB`. A missing channel follows hue and tone.
    struct Palette: Hashable {
        var head: String?
        var eye: String?
        var bg: String?

        var isEmpty: Bool { head == nil && eye == nil && bg == nil }

        /// Channels set here win; the others come from `base`.
        func inheriting(from base: Palette) -> Palette {
            Palette(head: head ?? base.head, eye: eye ?? base.eye, bg: bg ?? base.bg)
        }
    }

    var shape: Shape?
    var expression: Expression?
    var background: Background?
    /// Degrees in [0, 360).
    var hue: Double?
    var tone: Tone?
    var palette = Palette()
    var motion: Bool?

    var isEmpty: Bool {
        shape == nil && expression == nil && background == nil && hue == nil && tone == nil && palette.isEmpty
            && motion == nil
    }

    /// Fields set here win; the omitted ones come from `base`, palette channel by channel.
    func inheriting(from base: BotAppearance) -> BotAppearance {
        BotAppearance(
            shape: shape ?? base.shape, expression: expression ?? base.expression,
            background: background ?? base.background, hue: hue ?? base.hue, tone: tone ?? base.tone,
            palette: palette.inheriting(from: base.palette), motion: motion ?? base.motion)
    }

    var isValid: Bool {
        (hue.map(Self.isValidHue) ?? true)
            && [palette.head, palette.eye, palette.bg].allSatisfy { $0.map { BotLook.canonicalHex($0) == $0 } ?? true }
    }

    static func isValidHue(_ hue: Double) -> Bool { hue.isFinite && hue >= 0 && hue < 360 }

    /// A hue from a control, wrapped into [0, 360); nil when it is not a number.
    static func normalizedHue(_ value: Double) -> Double? {
        guard value.isFinite else { return nil }
        let wrapped = value.truncatingRemainder(dividingBy: 360)
        let hue = wrapped < 0 ? wrapped + 360 : wrapped
        return hue < 360 ? hue : 0
    }

    /// The fields `bots.update` takes; omitted fields are left out, never sent as null.
    var wireObject: [String: Any] {
        var object: [String: Any] = [:]
        if let shape { object["shape"] = shape.rawValue }
        if let expression { object["expression"] = expression.rawValue }
        if let background { object["background"] = background.rawValue }
        if let hue { object["hue"] = hue }
        if let tone { object["tone"] = tone.rawValue }
        if let motion { object["motion"] = motion }
        var colors: [String: Any] = [:]
        if let head = palette.head { colors["head"] = head }
        if let eye = palette.eye { colors["eye"] = eye }
        if let bg = palette.bg { colors["bg"] = bg }
        if !colors.isEmpty { object["palette"] = colors }
        return object
    }
}

extension BotAppearance: Decodable {
    private enum Keys: String, CodingKey { case shape, expression, background, hue, tone, palette, motion }
    private enum PaletteKeys: String, CodingKey { case head, eye, bg }

    /// Reading is lenient field by field: a value this app does not know (a newer shape, a
    /// non-canonical color) falls back to the seeded default instead of hiding the whole bot.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: Keys.self)
        func text(_ key: Keys) -> String? { (try? container.decodeIfPresent(String.self, forKey: key)) ?? nil }
        shape = text(.shape).flatMap(Shape.init(rawValue:))
        expression = text(.expression).flatMap(Expression.init(rawValue:))
        background = text(.background).flatMap(Background.init(rawValue:))
        tone = text(.tone).flatMap(Tone.init(rawValue:))
        hue = ((try? container.decodeIfPresent(Double.self, forKey: .hue)) ?? nil).flatMap { Self.isValidHue($0) ? $0 : nil }
        motion = (try? container.decodeIfPresent(Bool.self, forKey: .motion)) ?? nil
        if let colors = try? container.nestedContainer(keyedBy: PaletteKeys.self, forKey: .palette) {
            func color(_ key: PaletteKeys) -> String? {
                ((try? colors.decodeIfPresent(String.self, forKey: key)) ?? nil).flatMap { BotLook.canonicalHex($0) == $0 ? $0 : nil }
            }
            palette = Palette(head: color(.head), eye: color(.eye), bg: color(.bg))
        }
    }
}

/// A bot's saved look: a base appearance and optional per-state overrides that inherit omitted
/// fields from it. The seed is always the stable `bot.id`; nothing here changes identity, and no
/// runtime frame, timestamp, or current state is stored.
struct BotLook: Hashable {
    static let version = 1

    var base = BotAppearance()
    var states: [BotAvatarState: BotAppearance] = [:]

    /// The appearance shown in `state`: its override over the base.
    func resolved(for state: BotAvatarState) -> BotAppearance {
        (states[state] ?? BotAppearance()).inheriting(from: base)
    }

    var isValid: Bool { base.isValid && states.values.allSatisfy(\.isValid) }

    /// The complete object `bots.update` replaces the saved look with.
    var wireObject: [String: Any] {
        var object: [String: Any] = ["version": Self.version, "base": base.wireObject]
        if !states.isEmpty {
            object["states"] = Dictionary(uniqueKeysWithValues: states.map { ($0.key.rawValue, $0.value.wireObject) })
        }
        return object
    }

    /// `#rrggbb` or `rrggbb` as canonical `#RRGGBB`; nil for anything else.
    static func canonicalHex(_ text: String) -> String? {
        let trimmed = text.trimmingCharacters(in: .whitespaces)
        let digits = trimmed.hasPrefix("#") ? trimmed.dropFirst() : Substring(trimmed)
        guard digits.count == 6, digits.allSatisfy(\.isHexDigit) else { return nil }
        return "#" + digits.uppercased()
    }

    /// WCAG contrast ratio between two canonical colors, 1 through 21.
    static func contrastRatio(_ first: String, _ second: String) -> Double? {
        guard let a = luminance(first), let b = luminance(second) else { return nil }
        return (max(a, b) + 0.05) / (min(a, b) + 0.05)
    }

    private static func luminance(_ hex: String) -> Double? {
        guard let canonical = canonicalHex(hex), let value = UInt32(canonical.dropFirst(), radix: 16) else { return nil }
        func channel(_ shift: UInt32) -> Double {
            let c = Double((value >> shift) & 0xFF) / 255
            return c <= 0.03928 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4)
        }
        return 0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
    }
}

extension BotLook: Decodable {
    private enum Keys: String, CodingKey { case version, base, states }

    struct UnsupportedVersion: Error {}

    /// Strict on the envelope (version 1 with a base), lenient inside it; a state this app does
    /// not know is skipped.
    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: Keys.self)
        guard try container.decode(Int.self, forKey: .version) == Self.version else { throw UnsupportedVersion() }
        base = try container.decode(BotAppearance.self, forKey: .base)
        let raw = (try? container.decodeIfPresent([String: BotAppearance].self, forKey: .states)) ?? nil
        for (key, appearance) in raw ?? [:] {
            if let state = BotAvatarState(rawValue: key) { states[state] = appearance }
        }
    }

    /// A bot's `look` on the wire. One this app cannot read leaves the bot on its seeded default
    /// rather than failing the roster.
    struct Lenient: Decodable {
        let value: BotLook?

        init(from decoder: Decoder) throws {
            value = try? BotLook(from: decoder)
        }
    }
}
