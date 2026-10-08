import AppKit
import CryptoKit
import Sparkle

/// Sparkle verifies Beans archives; the signed ready manifest gates the release.
/// Automatic updates install on normal quit, never by an unattended restart.
@MainActor
final class Updater {
    static let shared = Updater()
    nonisolated static let didFinishCheck = Notification.Name("beans.updater.didFinishCheck")
    nonisolated static let isEnabled: Bool = {
        guard Bundle.main.bundleIdentifier == "ai.amoena.beans",
            Bundle.main.object(forInfoDictionaryKey: "SUFeedURL") as? String
                == "https://github.com/bloodf/beans/releases/latest/download/appcast.xml",
            let text = Bundle.main.object(forInfoDictionaryKey: "SUPublicEDKey") as? String,
            let key = Data(base64Encoded: text), key.count == 32
        else { return false }
        return true
    }()
    nonisolated static var currentVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "—"
    }

    private var controller: SPUStandardUpdaterController?
    private let delegate = UpdaterDelegate()
    private var updater: SPUUpdater? { controller?.updater }
    private init() {}
    private struct DrainLease {
        let id: String
        let pid: Int
        let port: Int
        let deadline: ContinuousClock.Instant
    }
    private struct DrainStatus: Decodable {
        let control: Bool
        let prepared: Bool
        let ready: Bool
        let expires_in: Int?
        let pid: Int
        let active: Int
        let jobs: Int
        let lease_id: String?
    }
    private var drainLease: DrainLease?
    private var drainToken: String?
    private var drainPreparation: Task<DrainStatus, Error>?
    private var drainCancellation: Task<Void, Error>?
    private var installationHandoff: Task<Void, Never>?
    private var authorizedRelease: BeansReadyRelease?
    private var authorizedRelay: String?

    /// A conservative monotonic deadline includes network time and suspension.
    private var hasLiveUpdateDrain: Bool {
        drainCancellation == nil && drainLease.map { ContinuousClock().now < $0.deadline } == true
            && drainLease?.port == Preferences.cliPort && AppStore.shared.isConnected
            && AppStore.shared.client.state == .connected
    }

    /// Supplied by the native owner, including the composer retained behind Settings.
    var canTerminateSafely: (() -> Bool)?
    var hasPendingInstallation: Bool { delegate.pendingInstallation }
    var canInstallPendingUpdate: Bool {
        guard let authorizedRelease else { return false }
        return hasLiveUpdateDrain && delegate.pendingInstallation
            && delegate.installingRelease == authorizedRelease
            && AppStore.shared.relayURL == authorizedRelay && canTerminateSafely?() == true
    }
    func validatePendingInstallation() async throws {
        authorizedRelease = nil
        authorizedRelay = nil
        installationHandoff?.cancel()
        installationHandoff = nil
        if let drainCancellation { try await drainCancellation.value }
        if drainToken != nil, drainLease == nil { try await cancelUpdateDrain() }
        guard Self.isEnabled,
            delegate.pendingInstallation, let selected = delegate.installingRelease,
            canTerminateSafely?() == true
        else { throw BeansReadyRelease.invalid("Finish bot work and save or send drafts before installing the update") }
        if drainLease != nil, !hasLiveUpdateDrain {
            throw BeansReadyRelease.invalid("The Runner update lease expired or its connection changed; installation is not authorized")
        }

        // Close admission before fetching metadata or observing work. Remember the token
        // before sending: a lost response may still have taken the lease.
        if drainToken == nil { drainToken = try CLILauncher.updateToken() }
        guard let token = drainToken else { throw BeansReadyRelease.invalid("The Runner update token is unavailable") }
        let port = Preferences.cliPort
        let requestedAt = ContinuousClock().now
        let preparation = Task { @MainActor in
            try await AppStore.shared.client.request(
                "update.prepare", ["token": token, "ttl": 1800], as: DrainStatus.self)
        }
        drainPreparation = preparation
        let prepared: DrainStatus
        do {
            prepared = try await preparation.value
            drainPreparation = nil
        } catch {
            drainPreparation = nil
            throw error
        }
        guard drainCancellation == nil, drainToken == token, Preferences.cliPort == port,
            delegate.pendingInstallation, delegate.installingRelease == selected
        else { throw BeansReadyRelease.invalid("The Beans installation was canceled") }
        guard prepared.control, prepared.prepared, prepared.ready, prepared.active == 0, prepared.jobs == 0,
            let remaining = prepared.expires_in, remaining > 0,
            let id = prepared.lease_id, !id.isEmpty,
            drainLease == nil || (drainLease?.id == id && drainLease?.pid == prepared.pid)
        else { throw BeansReadyRelease.invalid("Finish admitted Runner work before installing the update") }
        drainLease = DrainLease(id: id, pid: prepared.pid, port: port, deadline: requestedAt.advanced(by: .seconds(remaining)))

        let latest = try await BeansReadyRelease.fetch()
        guard latest == selected else {
            throw BeansReadyRelease.invalid("The selected update is no longer the ready Beans release")
        }
        let snapshot = try await AppStore.shared.client.request("bootstrap", as: Wire.Snapshot.self)
        guard snapshot.runningChatIds.isEmpty, snapshot.runningTurns?.isEmpty != false else {
            throw BeansReadyRelease.invalid("Finish bot work before installing the update")
        }
        try await BeansReadyRelease.verifyRelay(snapshot.relayUrl, required: selected.requiredProtocol)
        struct RelaySelection: Decodable { let relay_url: String? }
        let current = try await AppStore.shared.client.request("hello", as: RelaySelection.self)
        guard current.relay_url == snapshot.relayUrl else {
            throw BeansReadyRelease.invalid("The selected relay changed; check the update again")
        }
        let status = try await AppStore.shared.client.request("update.status", as: DrainStatus.self)
        guard hasLiveUpdateDrain, status.control, status.prepared, status.ready,
            status.pid == drainLease?.pid, status.active == 0, status.jobs == 0,
            let remaining = status.expires_in, remaining > 0,
            delegate.pendingInstallation, delegate.installingRelease == selected,
            AppStore.shared.relayURL == snapshot.relayUrl,
            canTerminateSafely?() == true
        else { throw BeansReadyRelease.invalid("The Runner drain or native quit guard is no longer ready") }
        authorizedRelease = selected
        authorizedRelay = snapshot.relayUrl
        // The caller consumes this authorization synchronously on the main actor.
    }

    func cancelUpdateDrain() async throws {
        authorizedRelease = nil
        authorizedRelay = nil
        installationHandoff?.cancel()
        installationHandoff = nil
        delegate.installationResumed = false
        drainLease = nil
        if let drainCancellation {
            try await drainCancellation.value
            return
        }
        guard let token = drainToken else { return }
        let cancellation = Task { @MainActor in
            // A prepare already sent may acquire its lease after cancellation starts.
            // Wait for its result before releasing; neither path can authorize meanwhile.
            if let preparation = self.drainPreparation { _ = await preparation.result }
            struct Cancelled: Decodable { let released: Bool }
            _ = try await AppStore.shared.client.request("update.cancel", ["token": token], as: Cancelled.self)
            self.drainToken = nil
        }
        drainCancellation = cancellation
        do {
            try await cancellation.value
            drainCancellation = nil
        } catch {
            drainCancellation = nil
            // Keep the token so no later attempt can proceed over an unreleased lease.
            throw error
        }
    }

    fileprivate func installationCycleFinished() {
        Task {
            do { try await cancelUpdateDrain() }
            catch { NSAlert(error: error).runModal() }
        }
    }


    func start() {
        guard Self.isEnabled, controller == nil else { return }
        controller = SPUStandardUpdaterController(
            startingUpdater: true, updaterDelegate: delegate, userDriverDelegate: nil)
        // Plist values are defaults only. Never overwrite a saved opt-out on launch.
        if automaticallyChecksForUpdates { delegate.prepareCheck(userInitiated: false) }
    }

    var canCheckForUpdates: Bool { updater?.canCheckForUpdates == true && !delegate.preparing }
    func checkForUpdates() { delegate.prepareCheck(userInitiated: true) }

    /// Invoke inline: no queued closure may outlive the lease or native draft guard.
    func resumePostponedInstallation() -> Bool {
        guard !delegate.installationResumed, let handler = delegate.postponedInstallation,
            canInstallPendingUpdate
        else { return false }
        delegate.installationResumed = true
        installationHandoff?.cancel()
        installationHandoff = Task {
            do { try await Task.sleep(for: .seconds(30)) }
            catch { return }
            // A continuation that never reaches termination must not leave admission closed.
            do { try await cancelUpdateDrain() }
            catch { NSAlert(error: error).runModal() }
        }
        handler()
        return true
    }

    var needsPostponedInstallationResume: Bool {
        delegate.postponedInstallation != nil && !delegate.installationResumed
    }

    var lastCheckDescription: String {
        guard let date = updater?.lastUpdateCheckDate else { return L("Never checked") }
        let formatter = DateFormatter()
        formatter.dateStyle = .medium
        formatter.timeStyle = .short
        formatter.doesRelativeDateFormatting = true
        return L("Last checked %@", formatter.string(from: date))
    }
    var automaticallyChecksForUpdates: Bool {
        get { updater?.automaticallyChecksForUpdates ?? false }
        set { updater?.automaticallyChecksForUpdates = newValue }
    }
    var automaticallyDownloadsUpdates: Bool {
        get { updater?.automaticallyDownloadsUpdates ?? false }
        set { updater?.automaticallyDownloadsUpdates = newValue }
    }
}

@MainActor
private final class UpdaterDelegate: NSObject, SPUUpdaterDelegate {
    private var ready: BeansReadyRelease?
    private var checkedAt: Date?
    private var preparation: Task<Void, Never>?
    var installingRelease: BeansReadyRelease?
    var preparing: Bool { preparation != nil }
    var pendingInstallation = false
    var postponedInstallation: (@MainActor () -> Void)?
    var installationResumed = false

    func prepareCheck(userInitiated: Bool) {
        guard preparation == nil else { return }
        preparation = Task {
            do {
                let release = try await BeansReadyRelease.fetch()
                ready = release
                checkedAt = Date()
                preparation = nil
                if userInitiated {
                    Updater.shared.performPreparedCheck()
                } else if Updater.shared.automaticallyChecksForUpdates {
                    Updater.shared.performPreparedBackgroundCheck()
                }
            } catch {
                ready = nil
                checkedAt = nil
                preparation = nil
                if userInitiated {
                    let alert = NSAlert(error: error)
                    alert.runModal()
                }
                NotificationCenter.default.post(name: Updater.didFinishCheck, object: nil)
            }
        }
    }

    func updater(_ updater: SPUUpdater, mayPerform updateCheck: SPUUpdateCheck) throws {
        guard ready != nil, let checkedAt, Date().timeIntervalSince(checkedAt) < 60 else {
            prepareCheck(userInitiated: false)
            throw BeansReadyRelease.invalid("Waiting for a signed, complete Beans release")
        }
    }

    func feedURLString(for updater: SPUUpdater) -> String? {
        ready?.baseURL.appendingPathComponent("appcast.xml").absoluteString
            ?? "https://github.com/bloodf/beans/releases/latest/download/appcast.xml"
    }

    func updater(_ updater: SPUUpdater, shouldProceedWithUpdate item: SUAppcastItem,
                 updateCheck: SPUUpdateCheck) throws {
        guard let ready, item.versionString == ready.version,
            item.fileURL == ready.baseURL.appendingPathComponent("Beans-\(ready.version).zip"),
            item.contentLength == ready.archiveSize,
            item.deltaUpdates?.isEmpty != false,
            (item.propertiesDictionary["beansProtocol"] as? String).flatMap(Int.init) == ready.requiredProtocol,
            item.releaseNotesURL == nil || item.releaseNotesURL == ready.baseURL.appendingPathComponent("Beans-\(ready.version).md"),
            item.fullReleaseNotesURL == nil
        else { throw BeansReadyRelease.invalid("The appcast does not match the signed Beans release") }
    }

    func updater(_ updater: SPUUpdater, willInstallUpdate item: SUAppcastItem) {
        pendingInstallation = true
        if installingRelease == nil { installingRelease = ready }
    }

    func updater(_ updater: SPUUpdater, willInstallUpdateOnQuit item: SUAppcastItem,
                 immediateInstallationBlock: @escaping () -> Void) -> Bool {
        pendingInstallation = true
        installingRelease = ready
        // Sparkle always installs on termination. Taking ownership suppresses impatient
        // restart prompts; deliberately do not invoke its immediate-relaunch block.
        return true
    }

    func updater(_ updater: SPUUpdater, shouldPostponeRelaunchForUpdate item: SUAppcastItem,
                 untilInvokingBlock installHandler: @escaping () -> Void) -> Bool {
        pendingInstallation = true
        installingRelease = ready
        postponedInstallation = { installHandler() }
        installationResumed = false
        // Enter the single AppKit quit path; it owns drain acquisition, every safety
        // check, and cancellation. Never authorize independently in this callback.
        Task { @MainActor in
            guard self.pendingInstallation, self.postponedInstallation != nil else { return }
            NSApp.terminate(nil)
        }
        return true
    }

    func updater(_ updater: SPUUpdater, didFinishUpdateCycleFor updateCheck: SPUUpdateCheck,
                 error: (any Error)?) {
        pendingInstallation = false
        postponedInstallation = nil
        installingRelease = nil
        installationResumed = false
        Updater.shared.installationCycleFinished()
        NotificationCenter.default.post(name: Updater.didFinishCheck, object: nil)
    }
}

extension Updater {
    fileprivate func performPreparedCheck() { controller?.checkForUpdates(nil) }
    fileprivate func performPreparedBackgroundCheck() { updater?.checkForUpdatesInBackground() }
}

private struct BeansReadyRelease: Sendable, Equatable {
    let version: String
    let baseURL: URL
    let archiveSize: UInt64
    let requiredProtocol: Int
    let manifestHash: String

    private struct Release: Decodable {
        let tag_name: String
        let draft: Bool
        let prerelease: Bool
    }
    private struct Manifest: Decodable {
        let schema: Int
        let version: String
        let revision: String
        let `protocol`: Int
        let artifacts: [Artifact]
    }
    private struct Artifact: Decodable {
        let name: String
        let sha256: String
        let size: UInt64
        let component: String
        let platform: String
        let version: String
    }

    static func invalid(_ message: String) -> NSError {
        NSError(domain: "Beans.Update", code: 1, userInfo: [NSLocalizedDescriptionKey: message])
    }
    private static func matches(_ value: String, _ pattern: String) -> Bool {
        value.range(of: pattern, options: .regularExpression) != nil
    }
    private static let session: URLSession = {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.httpShouldSetCookies = false
        configuration.httpCookieStorage = nil
        configuration.urlCredentialStorage = nil
        return URLSession(configuration: configuration)
    }()

    private static func download(_ url: URL, limit: Int) async throws -> Data {
        var request = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 30)
        request.setValue("Beans-Updater", forHTTPHeaderField: "User-Agent")
        let (bytes, response) = try await session.bytes(for: request)
        guard let http = response as? HTTPURLResponse, http.statusCode == 200 else {
            throw invalid("The Beans release is not ready")
        }
        var data = Data()
        for try await byte in bytes {
            guard data.count < limit else { throw invalid("Update metadata exceeds its size limit") }
            data.append(byte)
        }
        return data
    }
    static func fetch() async throws -> BeansReadyRelease {
        let decoder = JSONDecoder()
        let releases = try decoder.decode([Release].self, from: await download(
            URL(string: "https://api.github.com/repos/bloodf/beans/releases?per_page=100")!, limit: 1 << 20))
        let stable = "^(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)$"
        guard let release = releases.first(where: { !$0.draft && !$0.prerelease && $0.tag_name.hasPrefix("beans-v") }) else {
            throw invalid("No stable Beans release is published")
        }
        let version = String(release.tag_name.dropFirst("beans-v".count))
        guard matches(version, stable), matches(Updater.currentVersion, stable),
            version.compare(Updater.currentVersion, options: .numeric) != .orderedAscending
        else { throw invalid("Invalid or older Beans release version") }
        let base = URL(string: "https://github.com/bloodf/beans/releases/download/\(release.tag_name)/")!
        let body = try await download(base.appendingPathComponent("beans-update.json"), limit: 1 << 20)
        let signatureText = try await download(base.appendingPathComponent("beans-update.json.sig"), limit: 256)
        guard let text = String(data: signatureText, encoding: .utf8),
            let signature = Data(base64Encoded: text.trimmingCharacters(in: .whitespacesAndNewlines)), signature.count == 64,
            let keyText = Bundle.main.object(forInfoDictionaryKey: "SUPublicEDKey") as? String,
            let keyData = Data(base64Encoded: keyText),
            let key = try? Curve25519.Signing.PublicKey(rawRepresentation: keyData),
            key.isValidSignature(signature, for: body)
        else { throw invalid("Invalid Beans release signature") }
        let manifest = try decoder.decode(Manifest.self, from: body)
        guard manifest.schema == 1, manifest.version == version, manifest.protocol >= AppInfo.protocolVersion,
            matches(manifest.revision, "^[0-9a-f]{40}$"), !manifest.artifacts.isEmpty
        else { throw invalid("Invalid Beans release manifest") }
        var names = Set<String>()
        for artifact in manifest.artifacts {
            guard matches(artifact.name, "^[A-Za-z0-9][A-Za-z0-9._ -]*$"), !artifact.name.contains(".."),
                names.insert(artifact.name).inserted, matches(artifact.sha256, "^[0-9a-f]{64}$"),
                artifact.size > 0, artifact.size <= 1 << 30,
                !artifact.component.isEmpty, !artifact.platform.isEmpty,
                matches(artifact.version, stable)
            else { throw invalid("Invalid Beans release artifact") }
        }
        guard let archive = manifest.artifacts.first(where: { $0.name == "Beans-\(version).zip" }),
            let appcast = manifest.artifacts.first(where: { $0.name == "appcast.xml" }),
            manifest.artifacts.contains(where: { $0.name == "Beans-\(version).dmg" }),
            archive.version == version, appcast.version == version
        else { throw invalid("The Beans macOS release is incomplete") }
        let appcastBytes = try await download(base.appendingPathComponent("appcast.xml"), limit: 1 << 20)
        let hash = SHA256.hash(data: appcastBytes).map { String(format: "%02x", $0) }.joined()
        guard UInt64(appcastBytes.count) == appcast.size, hash == appcast.sha256 else {
            throw invalid("The appcast does not match the signed release")
        }
        return BeansReadyRelease(
            version: version, baseURL: base, archiveSize: archive.size, requiredProtocol: manifest.protocol,
            manifestHash: SHA256.hash(data: body).map { String(format: "%02x", $0) }.joined())
    }

    static func verifyRelay(_ selected: String?, required: Int) async throws {
        guard let selected, let url = URL(string: selected), url.host != nil,
            url.scheme == "https" || url.scheme == "http",
            url.user == nil, url.password == nil, url.query == nil, url.fragment == nil
        else { throw invalid("Select a Beans relay and upgrade it before installing this client") }
        let healthURL = url.appendingPathComponent("v1").appendingPathComponent("health")
        let data = try await download(healthURL, limit: 4096)
        let health = try Wire.RelayHealth.decode(data)
        guard health.supports(requiredProtocol: required) else {
            throw invalid("Upgrade the selected relay to Beans v2 protocol \(max(AppInfo.protocolVersion, required)) before installing this client")
        }
    }
}
