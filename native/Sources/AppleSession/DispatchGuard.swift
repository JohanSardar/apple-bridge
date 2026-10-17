import Foundation
import Darwin
import SourceSupport

actor DispatchGuard {
    private let file: FileHandle
    private(set) var outcomes: [String: JSON] = [:]

    init(home: URL) throws {
        try FileManager.default.createDirectory(at: home, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: home.path)
        let path = home.appendingPathComponent("dispatch-guard.log").path
        let descriptor = Darwin.open(path, O_RDWR | O_CREAT | O_APPEND | O_NOFOLLOW, 0o600)
        guard descriptor >= 0 else { throw SourceFailure.unavailable }
        var info = stat()
        guard fstat(descriptor, &info) == 0, info.st_uid == getuid(), info.st_mode & 0o777 == 0o600 else { Darwin.close(descriptor); throw SourceFailure.invalid }
        file = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
        var data = try file.readToEnd() ?? Data()
        if let last = data.last, last != 10 {
            let boundary = data.lastIndex(of: 10).map { $0 + 1 } ?? 0
            data = data.prefix(boundary)
            try file.truncate(atOffset: UInt64(boundary))
            try file.synchronize()
        }
        for line in data.split(separator: 10) {
            guard let entry = try? JSONDecoder().decode(JSON.self, from: Data(line)), !entry["command_id"].string.isEmpty else { throw SourceFailure.corrupt }
            let id = entry["command_id"].string
            if entry["t"].string == "Outcome" { outcomes[id] = entry }
            else {
                outcomes[id] = .object(["t": .string("Outcome"), "command_id": .string(id), "disposition": .string("indeterminate"), "evidence": .object(["reason": .string("dispatch_started_without_outcome"), "prepared": entry["prepared"]])])
            }
        }
        guard fsync(descriptor) == 0 else { throw SourceFailure.unavailable }
        let directory = Darwin.open(home.path, O_RDONLY)
        guard directory >= 0 else { throw SourceFailure.unavailable }
        defer { Darwin.close(directory) }
        guard fsync(directory) == 0 else { throw SourceFailure.unavailable }
    }

    func contains(_ id: String) -> Bool { outcomes[id] != nil }

    func append(_ value: JSON) throws {
        let bytes = try JSONEncoder().encode(value) + Data([10])
        try file.write(contentsOf: bytes)
        try file.synchronize()
        if value["t"].string == "Outcome" { outcomes[value["command_id"].string] = value }
        else {
            outcomes[value["command_id"].string] = .object(["t": .string("Outcome"), "command_id": value["command_id"], "disposition": .string("indeterminate"), "evidence": .object(["prepared": value["prepared"]])])
        }
    }
}
