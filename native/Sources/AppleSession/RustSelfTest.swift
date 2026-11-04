import Foundation
import Darwin
import MessagesSource
import NativeProtocol
import SourceSupport

actor RustSelfTest {
    static let shared = RustSelfTest()
    private var active = false
    private var reportURL: URL { PrivateReport.directory.appendingPathComponent("messages-selftest.json") }

    func run(force: Bool = false, includeUI: Bool = true) async throws -> JSON {
        guard !active else { throw SourceFailure.conflict }
        active = true
        defer { active = false }
        let home = PrivateReport.directory.appendingPathComponent("messages-selftest", isDirectory: true)
        try FileManager.default.createDirectory(at: home, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let lock = Darwin.open(home.appendingPathComponent("selftest.lock").path, O_CREAT | O_RDWR | O_NOFOLLOW, 0o600)
        guard lock >= 0 else { throw SourceFailure.unavailable }
        defer { Darwin.close(lock) }
        var info = stat()
        guard fstat(lock, &info) == 0, info.st_uid == getuid(), info.st_mode & 0o777 == 0o600 else { throw SourceFailure.invalid }
        guard flock(lock, LOCK_EX | LOCK_NB) == 0 else { throw SourceFailure.conflict }
        let build = ProcessInfo.processInfo.operatingSystemVersionString
        let run = "selftest_" + UUID().uuidString.replacingOccurrences(of: "-", with: "")
        let request = JSON.object(["run": .string(run), "os_build": .string(build), "force": .bool(force), "include_ui": .bool(includeUI)])
        try await serve(flag: "--selftest-messages", request: request)
        guard let data = try? Data(contentsOf: reportURL), let report = try? JSONDecoder().decode(JSON.self, from: data),
              report["run_id"].string == run || (!force && report["os_build"].string == build) else { throw SourceFailure.unavailable }
        return report
    }

    func cleanup(tag: String, report: String) async throws {
        guard !active else { throw SourceFailure.conflict }
        active = true
        defer { active = false }
        try await serve(flag: "--cleanup-messages", request: .object(["tag": .string(tag), "report": .string(report)]))
    }

    private func serve(flag: String, request: JSON) async throws {
        let requestText = String(decoding: try JSONEncoder().encode(request), as: UTF8.self)
        var pair: [Int32] = [-1, -1]
        guard socketpair(AF_UNIX, SOCK_STREAM, 0, &pair) == 0, fcntl(pair[0], F_SETFD, FD_CLOEXEC) == 0, fcntl(pair[1], F_SETFD, FD_CLOEXEC) == 0 else { throw SourceFailure.unavailable }
        let child: pid_t
        do { child = try Self.spawn(flag: flag, link: pair[1], request: requestText) }
        catch { Darwin.close(pair[0]); Darwin.close(pair[1]); throw error }
        Darwin.close(pair[1])
        let descriptor = pair[0]
        defer { Darwin.close(descriptor) }
        do {
            let link = try await Task.detached { try NativeSessionLink(descriptor: descriptor) }.value
            let commands = MessagesCommands(path: nil, labCommands: true)
            while true {
                let frame: JSON
                do { frame = try await link.receive() } catch { break }
                guard frame["t"].string == "Primitive" else { continue }
                try await Self.answer(frame, commands: commands, link: link)
            }
        } catch {
            Self.reap(child)
            throw error
        }
        Self.reap(child)
    }

    private static func answer(_ frame: JSON, commands: MessagesCommands, link: NativeSessionLink) async throws {
        let id = frame["id"]
        do {
            let operation = frame["op"].string
            let result: JSON = operation == "messages.reconcile" ? .object([:]) : try await commands.primitive(operation, args: frame["args"])
            try link.send(NativeSessionFrame.primitiveResult(id: id, result: result))
        } catch {
            let code: String
            switch error {
            case SourceFailure.conflict: code = "conflict"
            case SourceFailure.expired: code = "expired"
            case SourceFailure.unsupported: code = "unsupported"
            case SourceFailure.invalid, is MessagesSendSafetyFailure: code = "invalid"
            case SourceFailure.permissionMissing: code = "permission_missing"
            default: code = "unavailable"
            }
            try link.send(NativeSessionFrame.primitiveError(id: id, code: code))
        }
    }

    private static func spawn(flag: String, link: Int32, request: String) throws -> pid_t {
        let binary = ProcessInfo.processInfo.environment["AB_SESSION_BINARY"] ?? Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/ab-session").path
        guard link < 198, FileManager.default.isExecutableFile(atPath: binary) else { throw SourceFailure.unavailable }
        var actions: posix_spawn_file_actions_t?
        guard posix_spawn_file_actions_init(&actions) == 0 else { throw SourceFailure.unavailable }
        defer { posix_spawn_file_actions_destroy(&actions) }
        var attributes: posix_spawnattr_t?
        guard posix_spawnattr_init(&attributes) == 0 else { throw SourceFailure.unavailable }
        defer { posix_spawnattr_destroy(&attributes) }
        var defaults = sigset_t()
        sigfillset(&defaults)
        sigdelset(&defaults, SIGKILL)
        sigdelset(&defaults, SIGSTOP)
        var mask = sigset_t()
        sigemptyset(&mask)
        guard posix_spawnattr_setsigdefault(&attributes, &defaults) == 0,
              posix_spawnattr_setsigmask(&attributes, &mask) == 0,
              posix_spawnattr_setflags(&attributes, Int16(POSIX_SPAWN_CLOEXEC_DEFAULT | POSIX_SPAWN_SETSIGDEF | POSIX_SPAWN_SETSIGMASK)) == 0,
              posix_spawn_file_actions_adddup2(&actions, link, 198) == 0,
              posix_spawn_file_actions_addinherit_np(&actions, 2) == 0 else { throw SourceFailure.unavailable }
        let arguments = [binary, flag, "198", request]
        let pointers = arguments.map { strdup($0) } + [nil]
        defer { for pointer in pointers { free(pointer) } }
        let environment = ProcessInfo.processInfo.environment.map { strdup($0.key + "=" + $0.value) } + [nil]
        defer { for pointer in environment { free(pointer) } }
        var spawned: pid_t = 0
        let status = pointers.withUnsafeBufferPointer { argv in
            environment.withUnsafeBufferPointer { env in posix_spawn(&spawned, binary, &actions, &attributes, argv.baseAddress!, env.baseAddress!) }
        }
        guard status == 0 else { throw SourceFailure.unavailable }
        return spawned
    }

    private static func reap(_ child: pid_t) {
        var status: Int32 = 0
        for _ in 0..<100 {
            let result = waitpid(child, &status, WNOHANG)
            if result == child || result < 0 && errno != EINTR { return }
            usleep(50000)
        }
        kill(child, SIGTERM)
        for _ in 0..<50 {
            let result = waitpid(child, &status, WNOHANG)
            if result == child || result < 0 && errno != EINTR { return }
            usleep(10000)
        }
        kill(child, SIGKILL)
        while waitpid(child, &status, 0) < 0 && errno == EINTR {}
    }
}
