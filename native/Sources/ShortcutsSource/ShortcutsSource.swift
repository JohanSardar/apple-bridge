import Foundation
import CryptoKit
import SourceSupport
import Darwin

private final class Invocation: @unchecked Sendable {
    var pid: pid_t = 0
    var cancelled = false
    let lock = NSLock()
    var finished = false
    var timedOut = false
    func killIfRunning(timedOut: Bool = false) {
        lock.lock()
        defer { lock.unlock() }
        if !finished {
            cancelled = true
            self.timedOut = self.timedOut || timedOut
            if pid > 0 { killpg(pid, SIGKILL) }
        }
    }
    func start(_ executable: URL, arguments: [String], output: Int32) throws {
        lock.lock()
        defer { lock.unlock() }
        guard !cancelled else { throw CancellationError() }
        var actions: posix_spawn_file_actions_t?
        var attributes: posix_spawnattr_t?
        posix_spawn_file_actions_init(&actions)
        posix_spawnattr_init(&attributes)
        defer { posix_spawn_file_actions_destroy(&actions); posix_spawnattr_destroy(&attributes) }
        posix_spawn_file_actions_addopen(&actions, STDIN_FILENO, "/dev/null", O_RDONLY, 0)
        posix_spawn_file_actions_adddup2(&actions, output, STDOUT_FILENO)
        posix_spawn_file_actions_addopen(&actions, STDERR_FILENO, "/dev/null", O_WRONLY, 0)
        posix_spawnattr_setflags(&attributes, Int16(POSIX_SPAWN_SETPGROUP))
        posix_spawnattr_setpgroup(&attributes, 0)
        let argv = ([executable.path] + arguments).map { strdup($0) } + [nil]
        defer { argv.forEach { free($0) } }
        let environment = ProcessInfo.processInfo.environment.map { strdup($0.key + "=" + $0.value) } + [nil]
        defer { environment.forEach { free($0) } }
        let status = argv.withUnsafeBufferPointer { buffer in
            environment.withUnsafeBufferPointer { env in
                posix_spawn(&pid, executable.path, &actions, &attributes, UnsafeMutablePointer(mutating: buffer.baseAddress!), UnsafeMutablePointer(mutating: env.baseAddress!))
            }
        }
        guard status == 0 else { throw SourceFailure.unavailable }
    }
    func finish() -> Bool {
        lock.lock()
        if pid > 0 { killpg(pid, SIGKILL) }
        finished = true
        let result = timedOut
        lock.unlock()
        return result
    }
}

public actor ShortcutsSource: Source {
    public nonisolated let name = "shortcuts"
    public static let coverage: JSON = .object(["fields": .string("partial"), "missing": .array([.string("accepts input unavailable from CLI"), .string("installed pack version unverified"), .string("file blob input/output not qualified")])])
    private let executable: URL
    private var generation = UUID().uuidString
    public init(executable: URL = URL(fileURLWithPath: "/usr/bin/shortcuts")) { self.executable = executable }
    private func invoke(_ arguments: [String], timeout: Double, input: Data? = nil, running: Bool = false) async throws -> (Data, Int32, Bool) {
        let executable = executable
        let limit = running ? 32 * 1024 : 16 * 1024 * 1024
        let invocation = Invocation()
        let task = Task.detached {
            let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
            defer { try? FileManager.default.removeItem(at: directory) }
            let output = directory.appendingPathComponent("stdout")
            _ = FileManager.default.createFile(atPath: output.path, contents: nil, attributes: [.posixPermissions: 0o600])
            let handle = try FileHandle(forWritingTo: output)
            defer { try? handle.close() }
            var arguments = arguments
            let result = directory.appendingPathComponent("result")
            if running {
                arguments += ["--output-path", result.path]
                if let input {
                    let path = directory.appendingPathComponent("input")
                    try input.write(to: path)
                    try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: path.path)
                    arguments += ["--input-path", path.path]
                }
            }
            try invocation.start(executable, arguments: arguments, output: running ? FileHandle.nullDevice.fileDescriptor : handle.fileDescriptor)
            let monitor = DispatchSource.makeTimerSource(queue: .global())
            monitor.schedule(deadline: .now(), repeating: .milliseconds(50))
            monitor.setEventHandler {
                let paths = running ? [result] : [output]
                for path in paths {
                    if let size = try? path.resourceValues(forKeys: [.fileSizeKey]).fileSize, size > limit { invocation.killIfRunning() }
                }
            }
            monitor.resume()
            defer { monitor.cancel() }
            let deadline = DispatchWorkItem { invocation.killIfRunning(timedOut: true) }
            DispatchQueue.global().asyncAfter(deadline: .now() + timeout, execute: deadline)
            var waitStatus: Int32 = 0
            while waitpid(invocation.pid, &waitStatus, 0) < 0 && errno == EINTR {}
            let timedOut = invocation.finish()
            let status: Int32 = (waitStatus & 0x7f) == 0 ? (waitStatus >> 8) & 0xff : -1
            deadline.cancel()
            let path = running ? result : output
            if !FileManager.default.fileExists(atPath: path.path) { return (Data(), status, timedOut) }
            let size = try path.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
            guard size <= limit else { throw SourceFailure.unavailable }
            return (try Data(contentsOf: path), status, timedOut)
        }
        return try await withTaskCancellationHandler { try await task.value } onCancel: { invocation.killIfRunning() }
    }
    private func inventory(_ arguments: [String]) async throws -> [(String, String)] {
        let (data, status, _) = try await invoke(["list", "--show-identifiers"] + arguments, timeout: 15)
        guard status == 0, let text = String(data: data, encoding: .utf8) else { throw SourceFailure.unavailable }
        var seen = Set<String>()
        return try text.split(separator: "\n").map { line in
            guard line.hasSuffix(")"), let delimiter = line.range(of: " (", options: .backwards) else { throw SourceFailure.corrupt }
            let name = String(line[..<delimiter.lowerBound])
            let identifier = String(line[delimiter.upperBound...].dropLast())
            guard !name.isEmpty, UUID(uuidString: identifier) != nil, seen.insert(identifier.lowercased()).inserted else { throw SourceFailure.corrupt }
            return (name, identifier)
        }
    }
    public func open() async -> SourceReadiness {
        var coverage = Self.coverage.object
        do {
            let entries = try await inventory([])
            let names = entries.map { $0.0 }
            let required = ["AB Echo", "AB Clock", "AB Focus", "AB Pack v1"]
            if required.allSatisfy({ name in names.filter { $0 == name }.count == 1 }) {
                coverage["pack_version"] = .string("1")
                coverage["missing"] = .array([.string("accepts input unavailable from CLI"), .string("file blob input/output not qualified")])
            }
        } catch {
            coverage["missing"] = .array([.string("installed inventory unavailable"), .string("installed pack version unverified")])
        }
        return SourceReadiness(state: "ready", coverage: .object(coverage), capabilities: ["shortcuts.run"])
    }
    public func capture(_ resume: JSON, into sink: any ObservationSink) async throws {
        if !resume["generation"].string.isEmpty { generation = resume["generation"].string }
        var previous: [JSON]?
        while !Task.isCancelled {
            let started = timestamp()
            let shortcuts = try await inventory([])
            let folders = try await inventory(["--folders"])
            var membership: [String: String] = [:]
            for (folder, identifier) in folders {
                for (_, shortcut) in try await inventory(["--folder-name", folder]) {
                    guard membership[shortcut] == nil else { throw SourceFailure.corrupt }
                    membership[shortcut] = identifier
                }
            }
            var observations: [JSON] = []
            for (collection, entries) in [("shortcuts.folder", folders), ("shortcuts.shortcut", shortcuts)] {
                for (title, identifier) in entries {
                    var data: [String: JSON] = ["name": .string(title), "identifier": .string(identifier)]
                    if collection == "shortcuts.shortcut" {
                        data["accepts_input"] = .null
                        data["folder_id"] = membership[identifier].map { .object(["$ref": .object(["collection": .string("shortcuts.folder"), "native_key": .string($0)])]) } ?? .null
                    }
                    observations.append(.object(["kind": .string("upsert"), "collection": .string(collection), "native_key": .string(identifier), "scoped_alias": .string(name + "/" + collection + "/" + identifier), "data": .object(data)]))
                }
            }
            if previous != observations {
                let stamped = observations.map { observation -> JSON in
                    var value = observation.object
                    value["observed_at"] = .string(started)
                    return .object(value)
                }
                try await sink.emitCheckpoint(source: name, generation: generation, checkpoint: UUID().uuidString, observations: stamped, scanID: UUID().uuidString, scanStartedAt: started, scope: ["shortcuts.folder", "shortcuts.shortcut"], completionEvidence: "complete", blobHandles: [:], live: false)
                previous = observations
            }
            try await Task.sleep(for: .seconds(30))
        }
    }
    public func prepare(_ command: JSON) async throws -> Prepared {
        let action = command["action"]
        let args = action["args"]
        guard action["op"].string == "shortcuts.run", !args["shortcut"].string.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, args["shortcut"].string.utf8.count <= 1024, !args["shortcut"].string.hasPrefix("-"), args["timeout_ms"].number >= 0, args["timeout_ms"].number <= 300000, args["input"].string.utf8.count <= 16 * 1024 else { throw SourceFailure.invalid }
        if !args["input_base64"].string.isEmpty {
            guard args["input"].string.isEmpty, let data = Data(base64Encoded: args["input_base64"].string), data.count <= 16 * 1024 else { throw SourceFailure.invalid }
        }
        return Prepared(nativeState: .object(["route": .string("shortcuts_cli")]), fingerprint: command["request_hash"].string, action: action)
    }
    public func dispatch(_ prepared: Prepared) async throws -> JSON {
        let args = prepared.action["args"]
        let timeout = args["timeout_ms"].number == 0 ? 30 : args["timeout_ms"].number / 1000
        let input = args["input_base64"].string.isEmpty ? args["input"].string.data(using: .utf8) : Data(base64Encoded: args["input_base64"].string)
        let (data, status, timedOut) = try await invoke(["run", args["shortcut"].string], timeout: timeout, input: input, running: true)
        return .object(["disposition": .string(status == 0 ? "locally_observed" : "indeterminate"), "exit_status": .number(Double(status)), "timed_out": .bool(timedOut), "output_base64": .string(data.base64EncodedString()), "output_bytes": .number(Double(data.count)), "output_sha256": .string(SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()), "effect_scope": .string("runner completion only")])
    }
    public func resolve(_ command: String, evidence: JSON) async -> JSON {
        guard evidence["effect_scope"].string == "runner completion only",
              evidence["exit_status"] == .number(0), !evidence["timed_out"].bool,
              !evidence["prepared"]["request_hash"].string.isEmpty,
              evidence["prepared"]["action"]["op"].string == "shortcuts.run",
              let output = Data(base64Encoded: evidence["output_base64"].string), output.count <= 32 * 1024 else { return .null }
        var result = evidence.object
        result["observed"] = .bool(true)
        result["disposition"] = .string("locally_observed")
        return .object(result)
    }
}
