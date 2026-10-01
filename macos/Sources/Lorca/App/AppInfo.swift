import Foundation

enum AppInfo {
    static let name = Bundle.main.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String
        ?? "Beans"
    static let isDevelopment = Bundle.main.bundleIdentifier == "ai.amoena.beans.dev"
    static let defaultCLIHome = FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent(isDevelopment ? ".beans-dev" : ".beans")
    /// The relay a release build's CLI falls back to: the Beans relay on CortexOS. Beans Dev has none;
    /// the dev loop's relay on this Mac stands in.
    static let productionRelayURL = "https://cortex.tailfd052e.ts.net:8790"
    static let defaultCLIPort = isDevelopment ? 4865 : 4864
    static let cliCommand = isDevelopment
        ? "lorca serve --home ~/.beans-dev --port 4865"
        : "lorca serve --home ~/.beans --port 4864"
}
