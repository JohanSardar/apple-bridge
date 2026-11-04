import Foundation
import Darwin
import SourceSupport

final class SessionProxy: @unchecked Sendable {
    let descriptor: Int32
    let nativeDescriptor: Int32
    let factsDescriptor: Int32
    private let child: pid_t

    init(gateway: Int32, binary: String, home: String, effects: Bool, session: String? = nil) throws {
        var pair: [Int32] = [-1, -1]
        var native: [Int32] = [-1, -1]
        var facts: [Int32] = [-1, -1]
        func fail() -> SourceFailure {
            for descriptor in pair + native + facts where descriptor >= 0 { Darwin.close(descriptor) }
            return SourceFailure.unavailable
        }
        guard socketpair(AF_UNIX, SOCK_STREAM, 0, &pair) == 0 else { throw fail() }
        guard socketpair(AF_UNIX, SOCK_STREAM, 0, &native) == 0 else { throw fail() }
        guard socketpair(AF_UNIX, SOCK_STREAM, 0, &facts) == 0 else { throw fail() }
        guard (pair + native + facts).allSatisfy({ fcntl($0, F_SETFD, FD_CLOEXEC) == 0 }) else { throw fail() }
        var actions: posix_spawn_file_actions_t?
        guard posix_spawn_file_actions_init(&actions) == 0 else { throw fail() }
        defer { posix_spawn_file_actions_destroy(&actions) }
        guard gateway < 198, (pair + native + facts).allSatisfy({ $0 < 198 }) else { throw fail() }
        var attributes: posix_spawnattr_t?
        guard posix_spawnattr_init(&attributes) == 0 else { throw fail() }
        defer { posix_spawnattr_destroy(&attributes) }
        var defaults = sigset_t()
        sigfillset(&defaults)
        sigdelset(&defaults, SIGKILL)
        sigdelset(&defaults, SIGSTOP)
        var mask = sigset_t()
        sigemptyset(&mask)
        guard posix_spawnattr_setsigdefault(&attributes, &defaults) == 0,
              posix_spawnattr_setsigmask(&attributes, &mask) == 0,
              posix_spawnattr_setflags(&attributes, Int16(POSIX_SPAWN_CLOEXEC_DEFAULT | POSIX_SPAWN_SETSIGDEF | POSIX_SPAWN_SETSIGMASK)) == 0 else { throw fail() }
        var spawned: pid_t = 0
        var arguments = [binary, "--gateway-fd", "198", "--helper-fd", "199"]
        if let session {
            let owned = ProcessInfo.processInfo.environment["AB_SESSION_RUST_COMMANDS"] ?? "reminders,calendar,contacts,notes"
            arguments += ["--session-secret", session, "--session-home", home]
            if !owned.isEmpty { arguments += ["--rust-commands", owned] }
        }
        else {
            if effects { arguments += ["--effects-home", home] }
            if FileManager.default.fileExists(atPath: (home as NSString).appendingPathComponent("session/rust-capture.json")) { arguments += ["--capture-home", home] }
        }
        let pointers = arguments.map { strdup($0) } + [nil]
        defer { for pointer in pointers { free(pointer) } }
        let environment = ProcessInfo.processInfo.environment.filter { $0.key != "AB_NATIVE_FD" && $0.key != "AB_NATIVE_FACTS_FD" }.map { strdup($0.key + "=" + $0.value) } + [strdup("AB_NATIVE_FD=200"), strdup("AB_NATIVE_FACTS_FD=201"), nil]
        defer { for pointer in environment { free(pointer) } }
        let configured = posix_spawn_file_actions_adddup2(&actions, gateway, 198) == 0
            && posix_spawn_file_actions_adddup2(&actions, pair[1], 199) == 0
            && posix_spawn_file_actions_adddup2(&actions, native[1], 200) == 0
            && posix_spawn_file_actions_adddup2(&actions, facts[1], 201) == 0
            && posix_spawn_file_actions_addclose(&actions, native[0]) == 0
            && posix_spawn_file_actions_addclose(&actions, native[1]) == 0
            && posix_spawn_file_actions_addclose(&actions, facts[0]) == 0
            && posix_spawn_file_actions_addclose(&actions, facts[1]) == 0
            && posix_spawn_file_actions_addclose(&actions, pair[0]) == 0
            && posix_spawn_file_actions_addclose(&actions, gateway) == 0
            && posix_spawn_file_actions_addclose(&actions, pair[1]) == 0
        let status = configured ? pointers.withUnsafeBufferPointer { argv in
            environment.withUnsafeBufferPointer { env in posix_spawn(&spawned, binary, &actions, &attributes, argv.baseAddress!, env.baseAddress!) }
        } : EINVAL
        guard status == 0 else { throw fail() }
        Darwin.close(pair[1])
        Darwin.close(native[1])
        Darwin.close(facts[1])
        nativeDescriptor = native[0]
        factsDescriptor = facts[0]
        descriptor = pair[0]; child = spawned
    }

    func reap() {
        kill(child, SIGTERM)
        var status: Int32 = 0
        for _ in 0..<50 {
            let result = waitpid(child, &status, WNOHANG)
            if result == child || result < 0 && errno != EINTR { return }
            usleep(10000)
        }
        kill(child, SIGKILL)
        while waitpid(child, &status, 0) < 0 && errno == EINTR {}
    }
}
