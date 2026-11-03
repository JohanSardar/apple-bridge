import Foundation
import Darwin
import SourceSupport

final class SessionProxy: @unchecked Sendable {
    let descriptor: Int32
    let nativeDescriptor: Int32
    private let child: pid_t

    init(gateway: Int32, binary: String) throws {
        var pair: [Int32] = [-1, -1]
        guard socketpair(AF_UNIX, SOCK_STREAM, 0, &pair) == 0 else { throw SourceFailure.unavailable }
        var native: [Int32] = [-1, -1]
        guard socketpair(AF_UNIX, SOCK_STREAM, 0, &native) == 0 else {
            Darwin.close(pair[0]); Darwin.close(pair[1]); Darwin.close(native[0]); Darwin.close(native[1]); throw SourceFailure.unavailable
        }
        guard (pair + native).allSatisfy({ fcntl($0, F_SETFD, FD_CLOEXEC) == 0 }) else {
            Darwin.close(pair[0]); Darwin.close(pair[1]); Darwin.close(native[0]); Darwin.close(native[1]); throw SourceFailure.unavailable
        }
        var actions: posix_spawn_file_actions_t?
        guard posix_spawn_file_actions_init(&actions) == 0 else {
            Darwin.close(pair[0]); Darwin.close(pair[1]); Darwin.close(native[0]); Darwin.close(native[1]); throw SourceFailure.unavailable
        }
        defer { posix_spawn_file_actions_destroy(&actions) }
        guard gateway < 198, pair.allSatisfy({ $0 < 198 }), native.allSatisfy({ $0 < 198 }) else {
            Darwin.close(pair[0]); Darwin.close(pair[1]); Darwin.close(native[0]); Darwin.close(native[1]); throw SourceFailure.unavailable
        }
        var attributes: posix_spawnattr_t?
        guard posix_spawnattr_init(&attributes) == 0 else {
            Darwin.close(pair[0]); Darwin.close(pair[1]); Darwin.close(native[0]); Darwin.close(native[1]); throw SourceFailure.unavailable
        }
        defer { posix_spawnattr_destroy(&attributes) }
        guard posix_spawnattr_setflags(&attributes, Int16(POSIX_SPAWN_CLOEXEC_DEFAULT)) == 0 else {
            Darwin.close(pair[0]); Darwin.close(pair[1]); Darwin.close(native[0]); Darwin.close(native[1]); throw SourceFailure.unavailable
        }
        var spawned: pid_t = 0
        let arguments = [binary, "--gateway-fd", "198", "--helper-fd", "199"]
        let pointers = arguments.map { strdup($0) } + [nil]
        defer { for pointer in pointers { free(pointer) } }
        let environment = ProcessInfo.processInfo.environment.filter { $0.key != "AB_NATIVE_FD" }.map { strdup($0.key + "=" + $0.value) } + [strdup("AB_NATIVE_FD=200"), nil]
        defer { for pointer in environment { free(pointer) } }
        let configured = posix_spawn_file_actions_adddup2(&actions, gateway, 198) == 0
            && posix_spawn_file_actions_adddup2(&actions, pair[1], 199) == 0
            && posix_spawn_file_actions_adddup2(&actions, native[1], 200) == 0
            && posix_spawn_file_actions_addclose(&actions, native[0]) == 0
            && posix_spawn_file_actions_addclose(&actions, native[1]) == 0
            && posix_spawn_file_actions_addclose(&actions, pair[0]) == 0
            && posix_spawn_file_actions_addclose(&actions, gateway) == 0
            && posix_spawn_file_actions_addclose(&actions, pair[1]) == 0
        let status = configured ? pointers.withUnsafeBufferPointer { argv in
            environment.withUnsafeBufferPointer { env in posix_spawn(&spawned, binary, &actions, &attributes, argv.baseAddress!, env.baseAddress!) }
        } : EINVAL
        guard status == 0 else {
            Darwin.close(pair[0]); Darwin.close(pair[1]); Darwin.close(native[0]); Darwin.close(native[1]); throw SourceFailure.unavailable
        }
        Darwin.close(pair[1])
        Darwin.close(native[1])
        nativeDescriptor = native[0]
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
