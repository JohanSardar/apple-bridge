#if LAB
import Foundation
import Darwin

enum LabTCCProbe {
    static func run(_ output: String) throws {
        var sockets: [Int32] = [0, 0]
        guard socketpair(AF_UNIX, SOCK_STREAM, 0, &sockets) == 0 else { throw POSIXError(.EIO) }
        defer { close(sockets[0]); close(sockets[1]) }
        var actions: posix_spawn_file_actions_t?
        posix_spawn_file_actions_init(&actions)
        defer { posix_spawn_file_actions_destroy(&actions) }
        posix_spawn_file_actions_adddup2(&actions, sockets[1], 198)
        posix_spawn_file_actions_addclose(&actions, sockets[0])
        posix_spawn_file_actions_addclose(&actions, sockets[1])
        let executable = Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/identity-spikes").path
        let strings = [executable, "198"].map { value in value.withCString { strdup($0) } }
        defer { strings.forEach { free($0) } }
        var arguments = strings + [nil]
        var pid: pid_t = 0
        let status = posix_spawn(&pid, executable, &actions, nil, &arguments, environ)
        guard status == 0 else { throw POSIXError(POSIXErrorCode(rawValue: status) ?? .EIO) }
        close(sockets[1])
        sockets[1] = -1
        var descriptor = pollfd(fd: sockets[0], events: Int16(POLLIN), revents: 0)
        guard poll(&descriptor, 1, 30000) > 0 else {
            kill(pid, SIGKILL)
            waitpid(pid, nil, 0)
            throw POSIXError(.ETIMEDOUT)
        }
        var bytes = Data()
        var byte: UInt8 = 0
        while read(sockets[0], &byte, 1) == 1 {
            bytes.append(byte)
            if byte == 10 { break }
        }
        var childStatus: Int32 = 0
        waitpid(pid, &childStatus, 0)
        guard childStatus == 0, !bytes.isEmpty else { throw POSIXError(.EIO) }
        try bytes.write(to: URL(fileURLWithPath: output), options: .atomic)
    }
}
#endif
