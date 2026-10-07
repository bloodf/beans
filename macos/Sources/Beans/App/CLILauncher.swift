import Foundation
import Darwin

/// Main-thread presentation of the CLI lifecycle. Probing, spawning, and reading readiness
/// run on the worker's serial queue, independently of AppKit constructing the first window.
@MainActor
final class CLILauncher {
    enum Status: Equatable {
        case idle
        case probing
        case starting
        case running(external: Bool)
        case failed(String)

        var message: String {
            switch self {
            case .idle: L("Waiting to start the CLI")
            case .probing: L("Looking for the CLI…")
            case .starting: L("Starting the CLI…")
            case let .running(external): external ? L("Using the CLI already running on this computer") : L("CLI started by the app")
            case let .failed(reason): reason
            }
        }
    }

    private(set) var status: Status = .idle {
        didSet { if status != oldValue { onStatusChange?(status) } }
    }
    var onStatusChange: ((Status) -> Void)?
    var onReady: (() -> Void)?

    private var stopping = false
    private var requestedPort: Int?
    private var generation = UUID()
    private lazy var worker = CLILaunchWorker { [weak self] generation, event in
        DispatchQueue.main.async { [weak self] in
            guard let self, !self.stopping, self.generation == generation else { return }
            switch event {
            case .probing: self.status = .probing
            case .starting: self.status = .starting
            case let .ready(external):
                // The core establishes and validates the fresh home before native control writes.
                if ProcessInfo.processInfo.environment["BEANS_UPDATE_TOKEN_FILE"] == nil {
                    do { _ = try Self.ensureUpdateToken() }
                    catch { NSLog("Native update control unavailable: \(error.localizedDescription)") }
                }
                self.status = .running(external: external)
                self.onReady?()
            case let .failed(failure): self.status = .failed(failure.message)
            }
        }
    }

    var port: Int { Preferences.cliPort }

    static func locateBinary() -> URL? {
        CLILaunchWorker.locateBinary(environment: ProcessInfo.processInfo.environment, bundle: Bundle.main.bundleURL)
    }

    static var logURL: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Logs/\(AppInfo.name)/cli.log")
    }

    static var updateTokenURL: URL {
        AppInfo.defaultCLIHome.appendingPathComponent("native-update-token")
    }

    private static func openTokenDirectory() throws -> FileHandle {
        let descriptor = open(AppInfo.defaultCLIHome.path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
        guard descriptor >= 0 else { throw CocoaError(.fileReadNoPermission) }
        let directory = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
        var metadata = stat()
        guard fstat(descriptor, &metadata) == 0,
            metadata.st_mode & mode_t(S_IFMT) == mode_t(S_IFDIR),
            metadata.st_uid == 0 || metadata.st_uid == geteuid(),
            metadata.st_mode & 0o022 == 0
        else {
            try? directory.close()
            throw CocoaError(.fileReadNoPermission)
        }
        let markerDescriptor = openat(descriptor, "format.json", O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC)
        guard markerDescriptor >= 0 else {
            try? directory.close()
            throw CocoaError(.fileReadCorruptFile)
        }
        let marker = FileHandle(fileDescriptor: markerDescriptor, closeOnDealloc: true)
        defer { try? marker.close() }
        var markerMetadata = stat()
        do {
            guard fstat(markerDescriptor, &markerMetadata) == 0,
                markerMetadata.st_mode & mode_t(S_IFMT) == mode_t(S_IFREG),
                markerMetadata.st_size >= 0, markerMetadata.st_size <= 4096
            else { throw CocoaError(.fileReadCorruptFile) }
            let data = try marker.read(upToCount: 4097) ?? Data()
            struct FormatMarker: Decodable { let format: String }
            guard data.count <= 4096,
                try JSONDecoder().decode(FormatMarker.self, from: data).format == AppInfo.format
            else { throw CocoaError(.fileReadCorruptFile) }
        } catch {
            try? directory.close()
            throw error
        }
        return directory
    }

    private static func ensureUpdateToken() throws -> URL {
        let url = updateTokenURL
        // Native control never opens an inherited operator token or replaces existing bytes.
        let directory = try openTokenDirectory()
        defer { try? directory.close() }
        let descriptor = openat(directory.fileDescriptor, url.lastPathComponent,
            O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, S_IRUSR | S_IWUSR)
        if descriptor < 0 {
            guard errno == EEXIST else { throw CocoaError(.fileWriteUnknown) }
        } else {
            let handle = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
            defer { try? handle.close() }
            try handle.write(contentsOf: Data((UUID().uuidString + UUID().uuidString + "\n").utf8))
            try handle.synchronize()
        }
        _ = try updateToken()
        return url
    }

    static func updateToken() throws -> String {
        if let configured = ProcessInfo.processInfo.environment["BEANS_UPDATE_TOKEN_FILE"] {
            let message = configured.isEmpty
                ? "Native update installation is disabled by BEANS_UPDATE_TOKEN_FILE."
                : "Native update installation requires app-owned control; the configured operator token is not read."
            throw NSError(domain: "Beans.Update", code: 1, userInfo: [NSLocalizedDescriptionKey: message])
        }
        let url = updateTokenURL
        let directory = try openTokenDirectory()
        defer { try? directory.close() }
        // Check the opened inode, not a path that could be swapped between stat and read.
        // Nonblocking open also prevents a FIFO from hanging the main actor.
        let descriptor = openat(directory.fileDescriptor, url.lastPathComponent,
            O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC)
        guard descriptor >= 0 else { throw CocoaError(.fileReadNoPermission) }
        let handle = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
        defer { try? handle.close() }
        var metadata = stat()
        guard fstat(descriptor, &metadata) == 0,
            metadata.st_mode & mode_t(S_IFMT) == mode_t(S_IFREG),
            metadata.st_uid == 0 || metadata.st_uid == geteuid(),
            metadata.st_mode & 0o077 == 0,
            metadata.st_size >= 0, metadata.st_size <= 4096
        else { throw CocoaError(.fileReadNoPermission) }
        let data = try handle.read(upToCount: 4097) ?? Data()
        guard data.count <= 4096, let text = String(data: data, encoding: .utf8) else { throw CocoaError(.fileReadCorruptFile) }
        let token = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard token.unicodeScalars.count >= 32 else { throw CocoaError(.fileReadCorruptFile) }
        return token
    }

    func ensureRunning() {
        guard !stopping else { return }
        let port = port
        guard port == AppInfo.defaultCLIPort else {
            status = .failed(L("This build connects only to its isolated Beans CLI port %d. Restore that port in Settings › Advanced.", AppInfo.defaultCLIPort))
            return
        }
        if requestedPort != port {
            requestedPort = port
            generation = UUID()
        }
        var environment = ProcessInfo.processInfo.environment
        environment["RUST_LOG"] = environment["RUST_LOG"] ?? "beans=info"
        environment["BEANS_HOME"] = AppInfo.defaultCLIHome.path
        // Supply the fixed path now, but create no account-home files until core readiness.
        // An inherited explicit disable or operator setting remains unchanged.
        if environment["BEANS_UPDATE_TOKEN_FILE"] == nil {
            environment["BEANS_UPDATE_TOKEN_FILE"] = Self.updateTokenURL.path
        }
        if !AppInfo.isDevelopment, environment["BEANS_DEFAULT_RELAY_URL"] == nil, !AppInfo.productionRelayURL.isEmpty {
            environment["BEANS_DEFAULT_RELAY_URL"] = AppInfo.productionRelayURL
        }
        worker.ensureRunning(.init(
            generation: generation, port: port, environment: environment,
            bundle: Bundle.main.bundleURL, logURL: Self.logURL))
    }

    func stop() {
        stopping = true
        // Fence queued starts before returning: quitting must never leave a child that starts
        // after the main thread has already stopped observing it.
        worker.stop()
    }
}
