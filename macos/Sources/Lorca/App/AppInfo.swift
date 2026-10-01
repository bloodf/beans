import Foundation

enum AppInfo {
    static let name = Bundle.main.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String
        ?? "Beans"
    static let isDevelopment = Bundle.main.bundleIdentifier == "ai.amoena.beans.dev"
    static let defaultCLIHome = FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent(isDevelopment ? ".lorca-dev" : ".lorca")
    /// The relay a release build's CLI falls back to: the Beans relay on CortexOS. Beans Dev has none;
    /// the dev loop's relay on this Mac stands in.
    static let productionRelayURL = "https://cortex.tailfd052e.ts.net:8790"
    static let defaultCLIPort = isDevelopment ? 4863 : 4862
    static let cliCommand = isDevelopment
        ? "lorca serve --home ~/.lorca-dev --port 4863"
        : "lorca serve"
}
