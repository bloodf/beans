import AppKit
import JavaScriptCore

/// One drawn avatar frame from `avatarFrame`, in the 0–100 viewBox, y down. Numbers only: the
/// app builds `CGPath`s from it and never parses or decodes SVG while animating.
struct AvatarFrameData {
    struct CubicPath {
        var start: CGPoint
        /// c1x c1y c2x c2y x y, one entry per cubic.
        var segments: [[CGFloat]]

        func add(to path: CGMutablePath, transform: CGAffineTransform = .identity) {
            path.move(to: start, transform: transform)
            for s in segments where s.count == 6 {
                path.addCurve(
                    to: CGPoint(x: s[4], y: s[5]), control1: CGPoint(x: s[0], y: s[1]),
                    control2: CGPoint(x: s[2], y: s[3]), transform: transform)
            }
            path.closeSubpath()
        }

        func cgPath(transform: CGAffineTransform = .identity) -> CGPath {
            let path = CGMutablePath()
            add(to: path, transform: transform)
            return path
        }
    }

    struct Circle {
        var center: CGPoint
        var radius: CGFloat
    }

    var background: CubicPath
    var backgroundFill: String
    var backgroundOpacity: CGFloat
    /// Applies to core, extra, and petals.
    var body: CGAffineTransform
    var core: CubicPath
    var extra: CubicPath
    var petals: [Circle]
    var head: String
    var eyes: [CubicPath]
    /// Full composites, already including `body`.
    var eyeMatrices: [CGAffineTransform]
    var eye: String

    /// Petals as one path under the body matrix; empty slots (radius 0) add nothing.
    func petalPath() -> CGPath {
        let path = CGMutablePath()
        for petal in petals where petal.radius > 0 {
            path.addEllipse(
                in: CGRect(x: petal.center.x - petal.radius, y: petal.center.y - petal.radius, width: petal.radius * 2, height: petal.radius * 2),
                transform: body)
        }
        return path
    }
}

extension AvatarFrameData {
    struct Malformed: Error {}

    /// Reads `avatarFrame`'s JSON object; anything missing or of the wrong type is an error, not a
    /// guessed default.
    init(json: Any?) throws {
        guard let object = json as? [String: Any],
            let background = object["background"] as? [String: Any],
            let eyes = object["eyes"] as? [Any], eyes.count == 2,
            let eyeMatrices = object["eyeMatrices"] as? [Any], eyeMatrices.count == 2,
            let petals = object["petals"] as? [Any]
        else { throw Malformed() }
        self.background = try Self.path(background["path"])
        backgroundFill = try Self.text(background["fill"])
        backgroundOpacity = try Self.number(background["opacity"])
        body = try Self.matrix(object["body"])
        core = try Self.path(object["core"])
        extra = try Self.path(object["extra"])
        self.petals = try petals.map { item in
            guard let circle = item as? [String: Any] else { throw Malformed() }
            return Circle(
                center: CGPoint(x: try Self.number(circle["cx"]), y: try Self.number(circle["cy"])),
                radius: try Self.number(circle["r"]))
        }
        head = try Self.text(object["head"])
        self.eyes = try eyes.map(Self.path)
        self.eyeMatrices = try eyeMatrices.map(Self.matrix)
        eye = try Self.text(object["eye"])
    }

    private static func number(_ value: Any?) throws -> CGFloat {
        guard let number = value as? NSNumber, number.doubleValue.isFinite else { throw Malformed() }
        return CGFloat(number.doubleValue)
    }

    private static func text(_ value: Any?) throws -> String {
        guard let text = value as? String else { throw Malformed() }
        return text
    }

    private static func numbers(_ value: Any?, count: Int) throws -> [CGFloat] {
        guard let array = value as? [Any], array.count == count else { throw Malformed() }
        return try array.map { try number($0) }
    }

    /// SVG `matrix(a b c d e f)`, which is `CGAffineTransform`'s order.
    private static func matrix(_ value: Any?) throws -> CGAffineTransform {
        let m = try numbers(value, count: 6)
        return CGAffineTransform(a: m[0], b: m[1], c: m[2], d: m[3], tx: m[4], ty: m[5])
    }

    private static func path(_ value: Any?) throws -> CubicPath {
        guard let object = value as? [String: Any], let segments = object["segments"] as? [Any] else { throw Malformed() }
        let start = try numbers(object["start"], count: 2)
        return CubicPath(start: CGPoint(x: start[0], y: start[1]), segments: try segments.map { try numbers($0, count: 6) })
    }
}

/// The bundled `@beans/blobatar` geometry API in one local JavaScriptCore context: endpoint
/// geometry per bot, look, and state (cached here, as the API leaves to callers), numeric
/// morphs, and frames. Geometry stays a `JSValue` between calls; only frames cross into Swift.
@MainActor
enum AvatarGeometryBridge {
    /// One endpoint: what a bot looks like at rest in one state.
    struct Endpoint {
        let key: String
        let geometry: JSValue
        /// The resolved appearance's motion switch; off means a static portrait.
        let motion: Bool
    }

    /// The bundled script; isolated tests point it at a freshly built one before first use.
    static var scriptURL = Bundle.main.url(forResource: "blobatar.jsc", withExtension: "js")

    private static let context: JSContext = {
        guard let url = scriptURL,
            let script = try? String(contentsOf: url, encoding: .utf8),
            let context = JSContext()
        else {
            preconditionFailure("Bundled Blobatar geometry missing")
        }
        context.evaluateScript(script)
        precondition(context.exception == nil, "Bundled Blobatar geometry failed: \(context.exception!)")
        // An app bundle carrying the old generator-only script: say which resource is stale.
        for name in ["botAvatarGeometry", "interpolateAvatarGeometry", "avatarMorphProgress", "avatarFrame", "resolveBotAppearance", "botAppearanceContrast", "validateBotLook"] {
            precondition(context.objectForKeyedSubscript(name)?.isObject == true, "blobatar.jsc.js lacks \(name); rebuild packages/beans-blobatar/dist")
        }
        return context
    }()

    private static var endpoints: [String: Endpoint] = [:]
    /// Frames computed during the current clock tick, so every view showing one endpoint at one
    /// time shares a single JavaScript call.
    private static var tickFrames: [String: AvatarFrameData] = [:]
    private static var tickTime: CFTimeInterval = -1
    // ponytail: unbounded until the limit, then cleared; LRU if rosters grow past it.
    private static let endpointLimit = 512

    private static func call(_ name: String, _ arguments: [Any]) -> JSValue {
        let result = context.objectForKeyedSubscript(name).call(withArguments: arguments)
        if let exception = context.exception {
            context.exception = nil
            preconditionFailure("Blobatar \(name) failed: \(exception)")
        }
        return result!
    }

    /// The look as the JavaScript API takes it; null is the seeded default.
    private static func lookArgument(_ look: BotLook?) -> Any { look?.wireObject ?? NSNull() }

    static func key(id: String, look: BotLook?, state: BotAvatarState) -> String {
        let json = look.flatMap { try? JSONSerialization.data(withJSONObject: $0.wireObject, options: [.sortedKeys]) }
        return "\(id)\n\(json.map { String(decoding: $0, as: UTF8.self) } ?? "null")\n\(state.rawValue)"
    }

    static func endpoint(id: String, look: BotLook?, state: BotAvatarState) -> Endpoint {
        let key = key(id: id, look: look, state: state)
        if let cached = endpoints[key] { return cached }
        if endpoints.count >= endpointLimit { endpoints.removeAll() }
        let geometry = call("botAvatarGeometry", [id, lookArgument(look), state.rawValue])
        let endpoint = Endpoint(key: key, geometry: geometry, motion: geometry.forProperty("motion").toBool())
        endpoints[key] = endpoint
        return endpoint
    }

    static func interpolate(from: JSValue, to: JSValue, t: Double) -> JSValue {
        call("interpolateAvatarGeometry", [from, to, t])
    }

    static func morphProgress(elapsed: CFTimeInterval, toward: JSValue) -> (t: Double, done: Bool) {
        let progress = call("avatarMorphProgress", [elapsed * 1000, toward])
        return (progress.forProperty("t").toDouble(), progress.forProperty("done").toBool())
    }

    /// `amp` 0 is the static frame: Reduce Motion, motion off, or a portrait not on screen.
    static func frame(of geometry: JSValue, time: CFTimeInterval, amp: Double) -> AvatarFrameData {
        do {
            return try AvatarFrameData(json: call("avatarFrame", [geometry, time * 1000, amp]).toObject())
        } catch {
            preconditionFailure("Blobatar avatarFrame returned a malformed frame")
        }
    }

    /// A settled endpoint's frame, shared for one tick (or, static, for good).
    static func frame(of endpoint: Endpoint, time: CFTimeInterval, amp: Double) -> AvatarFrameData {
        if amp == 0 {
            let key = endpoint.key + "\nstatic"
            if let frame = staticFrames[key] { return frame }
            if staticFrames.count >= endpointLimit { staticFrames.removeAll() }
            let frame = frame(of: endpoint.geometry, time: 0, amp: 0)
            staticFrames[key] = frame
            return frame
        }
        if time != tickTime {
            tickTime = time
            tickFrames.removeAll(keepingCapacity: true)
        }
        if let frame = tickFrames[endpoint.key] { return frame }
        let frame = frame(of: endpoint.geometry, time: time, amp: amp)
        tickFrames[endpoint.key] = frame
        return frame
    }

    private static var staticFrames: [String: AvatarFrameData] = [:]

    /// The appearance a bot shows in `state`, every field filled (seeded where the look omits
    /// it), and its WCAG contrast ratios, for the Look sheet.
    static func resolved(id: String, look: BotLook?, state: BotAvatarState) -> (appearance: [String: Any], eyeOnHead: Double, headOnBg: Double) {
        let resolved = call("resolveBotAppearance", [id, lookArgument(look), state.rawValue])
        let contrast = call("botAppearanceContrast", [resolved])
        return (
            resolved.toDictionary() as? [String: Any] ?? [:],
            contrast.forProperty("eyeOnHead").toDouble(), contrast.forProperty("headOnBg").toDouble()
        )
    }

    /// The bundled validator's verdict on a draft before it is sent; nil when valid.
    static func validationError(_ look: BotLook) -> String? {
        let result = call("validateBotLook", [look.wireObject])
        return result.forProperty("ok").toBool() ? nil : result.forProperty("error").toString()
    }
}

/// `#RRGGBB` colors as `CGColor`s, made once each.
@MainActor
enum AvatarColors {
    private static var colors: [String: CGColor] = [:]

    static func color(_ hex: String) -> CGColor {
        if let color = colors[hex] { return color }
        let value = BotLook.canonicalHex(hex).flatMap { UInt32($0.dropFirst(), radix: 16) } ?? 0
        let color = CGColor(
            srgbRed: CGFloat((value >> 16) & 0xFF) / 255, green: CGFloat((value >> 8) & 0xFF) / 255,
            blue: CGFloat(value & 0xFF) / 255, alpha: 1)
        colors[hex] = color
        return color
    }
}
