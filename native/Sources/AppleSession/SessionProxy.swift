import Foundation
import Darwin
import SourceSupport

final class SessionProxy: @unchecked Sendable {
    let descriptor: Int32
    private let child: pid_t

    init(gateway: Int32, binary: String, home: String, session: String) throws {
        var pair: [Int32] = [-1, -1]
        func fail() -> SourceFailure {
            for descriptor in pair where descriptor >= 0 { Darwin.close(descriptor) }
            return SourceFailure.unavailable
        }
        guard socketpair(AF_UNIX, SOCK_STREAM, 0, &pair) == 0 else { throw fail() }
        guard pair.allSatisfy({ fcntl($0, F_SETFD, FD_CLOEXEC) == 0 }) else { throw fail() }
        var actions: posix_spawn_file_actions_t?
        guard posix_spawn_file_actions_init(&actions) == 0 else { throw fail() }
        defer { posix_spawn_file_actions_destroy(&actions) }
        guard gateway < 198, pair.allSatisfy({ $0 < 198 }) else { throw fail() }
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
        let arguments = [binary, "--gateway-fd", "198", "--helper-fd", "199", "--session-secret", session, "--session-home", home]
        let pointers = arguments.map { strdup($0) } + [nil]
        defer { for pointer in pointers { free(pointer) } }
        let environment = ProcessInfo.processInfo.environment.map { strdup($0.key + "=" + $0.value) } + [nil]
        defer { for pointer in environment { free(pointer) } }
        let configured = posix_spawn_file_actions_adddup2(&actions, gateway, 198) == 0
            && posix_spawn_file_actions_adddup2(&actions, pair[1], 199) == 0
            && posix_spawn_file_actions_addclose(&actions, pair[0]) == 0
            && posix_spawn_file_actions_addclose(&actions, gateway) == 0
            && posix_spawn_file_actions_addclose(&actions, pair[1]) == 0
        let status = configured ? pointers.withUnsafeBufferPointer { argv in
            environment.withUnsafeBufferPointer { env in posix_spawn(&spawned, binary, &actions, &attributes, argv.baseAddress!, env.baseAddress!) }
        } : EINVAL
        guard status == 0 else { throw fail() }
        Darwin.close(pair[1])
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
