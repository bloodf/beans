import Foundation
import Darwin

/// Read-only local transport credential. Update-control tokens are independent.
enum ServeToken {
    static func read(home: URL = AppInfo.defaultCLIHome) throws -> String {
        var components = home.pathComponents
        var systemAlias: stat?
        if home.path == "/var" || home.path.hasPrefix("/var/") {
            var link = stat()
            guard lstat("/var", &link) == 0, link.st_uid == 0,
                try FileManager.default.destinationOfSymbolicLink(atPath: "/var") == "private/var"
            else { throw CocoaError(.fileReadNoPermission) }
            systemAlias = link
            components = URL(fileURLWithPath: "/private" + home.path).pathComponents
        }
        var parent = open("/", O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
        guard parent >= 0 else { throw CocoaError(.fileReadNoPermission) }
        var retained: [(Int32, String, stat)] = []
        defer { for entry in retained { close(entry.0) } }
        defer { close(parent) }
        for component in components where component != "/" {
            guard component != ".." else { throw CocoaError(.fileReadNoPermission) }
            let next = openat(parent, component, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
            guard next >= 0 else { throw CocoaError(.fileReadNoPermission) }
            var identity = stat()
            guard fstat(next, &identity) == 0 else { close(next); throw CocoaError(.fileReadNoPermission) }
            if systemAlias != nil && retained.count < 2 {
                guard identity.st_uid == 0, identity.st_mode & 0o022 == 0 else { close(next); throw CocoaError(.fileReadNoPermission) }
            }
            retained.append((parent, component, identity))
            parent = next
        }
        var directory = stat()
        guard fstat(parent, &directory) == 0,
            directory.st_uid == 0 || directory.st_uid == geteuid(), directory.st_mode & 0o022 == 0
        else { throw CocoaError(.fileReadNoPermission) }
        func verify() throws {
            if let expected = systemAlias {
                var current = stat()
                guard lstat("/var", &current) == 0, current.st_uid == 0,
                    current.st_dev == expected.st_dev, current.st_ino == expected.st_ino,
                    try FileManager.default.destinationOfSymbolicLink(atPath: "/var") == "private/var"
                else { throw CocoaError(.fileReadNoPermission) }
            }
            for (parent, name, expected) in retained {
                var current = stat()
                guard fstatat(parent, name, &current, AT_SYMLINK_NOFOLLOW) == 0,
                    current.st_dev == expected.st_dev, current.st_ino == expected.st_ino,
                    current.st_mode & mode_t(S_IFMT) == mode_t(S_IFDIR)
                else { throw CocoaError(.fileReadNoPermission) }
            }
        }
        func record(_ name: String, privateFile: Bool) throws -> Data {
            let fd = openat(parent, name, O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC)
            guard fd >= 0 else { throw CocoaError(.fileReadNoPermission) }
            let file = FileHandle(fileDescriptor: fd, closeOnDealloc: true)
            defer { try? file.close() }
            var info = stat()
            guard fstat(fd, &info) == 0, info.st_mode & mode_t(S_IFMT) == mode_t(S_IFREG),
                info.st_uid == 0 || info.st_uid == geteuid(), info.st_mode & (privateFile ? 0o077 : 0o022) == 0,
                info.st_size >= 0, info.st_size <= 4096
            else { throw CocoaError(.fileReadNoPermission) }
            try verify()
            let data = try file.read(upToCount: 4097) ?? Data()
            guard data.count <= 4096 else { throw CocoaError(.fileReadCorruptFile) }
            var current = stat()
            guard fstatat(parent, name, &current, AT_SYMLINK_NOFOLLOW) == 0,
                current.st_dev == info.st_dev, current.st_ino == info.st_ino
            else { throw CocoaError(.fileReadNoPermission) }
            try verify()
            return data
        }
        struct Marker: Decodable { let format: String }
        guard try JSONDecoder().decode(Marker.self, from: record("format.json", privateFile: false)).format == AppInfo.format
        else { throw CocoaError(.fileReadCorruptFile) }
        let data = try record("serve-token", privateFile: true)
        guard var token = String(data: data, encoding: .utf8) else { throw CocoaError(.fileReadCorruptFile) }
        if token.hasSuffix("\n") { token.removeLast() }
        guard token.utf8.count == 43,
            let raw = Data(base64Encoded: token.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/") + "="), raw.count == 32,
            raw.base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: "=", with: "") == token
        else { throw CocoaError(.fileReadCorruptFile) }
        return token
    }
}
