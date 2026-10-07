import Foundation

enum AppInfo {
    static let name = Bundle.main.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String
        ?? "Beans"
    static let isDevelopment = Bundle.main.bundleIdentifier == "ai.amoena.beans.dev"
    static let defaultCLIHome = FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent(isDevelopment ? ".beans-dev-v2" : ".beans-v2")
    /// A release bundle may supply its relay without putting deployment addresses in source.
    static let productionRelayURL = (Bundle.main.object(forInfoDictionaryKey: "BeansRelayURL") as? String)?
        .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    static let format = "beans-v2"
    static let protocolVersion = 5
    static let defaultCLIPort = isDevelopment ? 4875 : 4874
    static let cliCommand = isDevelopment
        ? "beans serve --home ~/.beans-dev-v2 --port 4875"
        : "beans serve --home ~/.beans-v2 --port 4874"
}
