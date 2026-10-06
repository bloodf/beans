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
        return directory
    }

    private static func ensureUpdateToken() throws -> URL {
        let url = updateTokenURL
        // Native control never opens an inherited operator token or replaces existing bytes.
        if mkdir(AppInfo.defaultCLIHome.path, mode_t(S_IRWXU)) != 0, errno != EEXIST {
            throw CocoaError(.fileWriteUnknown)
        }
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
        if let configured = ProcessInfo.processInfo.environment["LORCA_UPDATE_TOKEN_FILE"] {
            let message = configured.isEmpty
                ? "Native update installation is disabled by LORCA_UPDATE_TOKEN_FILE."
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
        if requestedPort != port {
            requestedPort = port
            generation = UUID()
        }
        var environment = ProcessInfo.processInfo.environment
        environment["RUST_LOG"] = environment["RUST_LOG"] ?? "lorca=info"
        environment["LORCA_HOME"] = AppInfo.defaultCLIHome.path
        // Preserve an inherited value exactly, including an explicit empty disable.
        // Provision native control only when absent; failure does not prevent startup.
        if environment["LORCA_UPDATE_TOKEN_FILE"] == nil, let token = try? Self.ensureUpdateToken() {
            environment["LORCA_UPDATE_TOKEN_FILE"] = token.path
        }
        if !AppInfo.isDevelopment, environment["LORCA_DEFAULT_RELAY_URL"] == nil, !AppInfo.productionRelayURL.isEmpty {
            environment["LORCA_DEFAULT_RELAY_URL"] = AppInfo.productionRelayURL
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
