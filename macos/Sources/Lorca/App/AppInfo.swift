import Foundation

enum AppInfo {
    static let name = Bundle.main.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String
        ?? "Beans"
    static let isDevelopment = Bundle.main.bundleIdentifier == "ai.amoena.beans.dev"
    static let defaultCLIHome = FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent(isDevelopment ? ".beans-dev" : ".beans")
    /// A release bundle may supply its relay without putting deployment addresses in source.
    static let productionRelayURL = (Bundle.main.object(forInfoDictionaryKey: "BeansRelayURL") as? String)?
        .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    static let defaultCLIPort = isDevelopment ? 4865 : 4864
    static let cliCommand = isDevelopment
        ? "lorca serve --home ~/.beans-dev --port 4865"
        : "lorca serve --home ~/.beans --port 4864"
}
