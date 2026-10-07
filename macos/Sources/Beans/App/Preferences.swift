import AppKit

enum Preferences {
    private enum Key {
        static let hadIdentity = "beans-v2.hadIdentity"
        static let selection = "beans-v2.selection"
        static let showsInspector = "beans-v2.showsInspector"
        static let sendOnReturn = "beans-v2.sendOnReturn"
        static let relayURL = "beans-v2.relayURL"
        static let cliPort = "beans-v2.cliPort"
        static let showTimestamps = "beans-v2.showTimestamps"
        static let dictationLanguage = "beans-v2.dictationLanguage"
        static let appearance = "beans-v2.appearance"
    }

    /// The look picked in Settings › General, in the order its pop-up lists them.
    enum Appearance: String, CaseIterable {
        case system, light, dark

        /// What `NSApp.appearance` takes; nil follows the system.
        var nsAppearance: NSAppearance? {
            switch self {
            case .system: nil
            case .light: NSAppearance(named: .aqua)
            case .dark: NSAppearance(named: .darkAqua)
            }
        }
    }

    private static let defaults = UserDefaults.standard

    /// Whether the CLI's last answer had an identity. Launch opens the main window at once when it
    /// did, and otherwise waits for the answer, so a fresh install goes straight to onboarding.
    static var hadIdentity: Bool {
        get { defaults.bool(forKey: Key.hadIdentity) }
        set { defaults.set(newValue, forKey: Key.hadIdentity) }
    }

    /// Persisted so a dev-mode relaunch lands back on the same conversation.
    static var selection: String? {
        get { defaults.string(forKey: Key.selection) }
        set { defaults.set(newValue, forKey: Key.selection) }
    }

    static var showsInspector: Bool {
        get { defaults.object(forKey: Key.showsInspector) as? Bool ?? true }
        set { defaults.set(newValue, forKey: Key.showsInspector) }
    }

    static var sendOnReturn: Bool {
        get { defaults.object(forKey: Key.sendOnReturn) as? Bool ?? true }
        set { defaults.set(newValue, forKey: Key.sendOnReturn) }
    }

    static var showTimestamps: Bool {
        get { defaults.object(forKey: Key.showTimestamps) as? Bool ?? true }
        set { defaults.set(newValue, forKey: Key.showTimestamps) }
    }

    /// Applied at launch, before any window shows.
    static var appearance: Appearance {
        get { defaults.string(forKey: Key.appearance).flatMap(Appearance.init(rawValue:)) ?? .system }
        set { defaults.set(newValue.rawValue, forKey: Key.appearance) }
    }

    /// Speech recognizer locale identifier; nil follows the system's preferred languages.
    static var dictationLanguage: String? {
        get { defaults.string(forKey: Key.dictationLanguage) }
        set { defaults.set(newValue, forKey: Key.dictationLanguage) }
    }

    /// The language the app's own words are in ("en", "zh-Hans"); nil follows the system. It is
    /// the app's `AppleLanguages` default, the one System Settings › Language & Region writes
    /// per app, so either place changes it and the other shows it. Read at launch.
    static var appLanguage: String? {
        get {
            let domain = Bundle.main.bundleIdentifier.flatMap { defaults.persistentDomain(forName: $0) }
            return (domain?["AppleLanguages"] as? [String])?.first
        }
        set {
            if let newValue { defaults.set([newValue], forKey: "AppleLanguages") } else { defaults.removeObject(forKey: "AppleLanguages") }
        }
    }

    static var relayURL: String {
        get { defaults.string(forKey: Key.relayURL) ?? "" }
        set { defaults.set(newValue, forKey: Key.relayURL) }
    }

    static var cliPort: Int {
        get {
            let stored = defaults.integer(forKey: Key.cliPort)
            return stored == 0 ? AppInfo.defaultCLIPort : stored
        }
        set { defaults.set(newValue, forKey: Key.cliPort) }
    }

    static func reset() {
        for key in [Key.hadIdentity, Key.selection] {
            defaults.removeObject(forKey: key)
        }
    }
}
