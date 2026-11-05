import Darwin
import Foundation

public enum TrustedChild {
    public static let sessionName = "ab-session"
    public static let bridgeName = "bridge"
    public static let gatewayIdentifier = "bridge"

    public static func session() throws -> String {
        #if LAB
        if let binary = ProcessInfo.processInfo.environment["AB_SESSION_BINARY"] { return binary }
        return bundled(sessionName)
        #else
        return try verified(sessionName)
        #endif
    }

    public static func bridge(labExecutable: String? = nil) throws -> String {
        #if LAB
        return labExecutable ?? bundled(bridgeName)
        #else
        return try verified(bridgeName)
        #endif
    }

    public static func verifyGateway(_ descriptor: Int32) throws {
        #if !LAB
        try CodeIdentity.verifySocketPeer(descriptor, identifier: gatewayIdentifier)
        #endif
    }

    public static func verifySessionParent() throws {
        #if !LAB
        try CodeIdentity.verifyProcess(pid: getppid(), identifier: sessionName)
        #endif
    }

    public static var spawnFlags: Int32 {
        #if LAB
        0
        #else
        POSIX_SPAWN_START_SUSPENDED
        #endif
    }

    public static func admit(_ pid: pid_t, identifier: String) throws {
        #if !LAB
        do { try CodeIdentity.verifyProcess(pid: pid, identifier: identifier) }
        catch {
            kill(pid, SIGKILL)
            var status: Int32 = 0
            while waitpid(pid, &status, 0) < 0 && errno == EINTR {}
            throw error
        }
        kill(pid, SIGCONT)
        #endif
    }

    public static func capture(_ path: String, identifier: String, arguments: [String], timeout: TimeInterval? = nil) -> Data? {
        var pipes: [Int32] = [-1, -1]
        guard pipe(&pipes) == 0 else { return nil }
        defer { if pipes[0] >= 0 { Darwin.close(pipes[0]) } }
        var actions: posix_spawn_file_actions_t?
        var attributes: posix_spawnattr_t?
        guard posix_spawn_file_actions_init(&actions) == 0 else { Darwin.close(pipes[1]); return nil }
        defer { posix_spawn_file_actions_destroy(&actions) }
        guard posix_spawnattr_init(&attributes) == 0 else { Darwin.close(pipes[1]); return nil }
        defer { posix_spawnattr_destroy(&attributes) }
        let configured = posix_spawnattr_setflags(&attributes, Int16(POSIX_SPAWN_CLOEXEC_DEFAULT | spawnFlags)) == 0
            && posix_spawn_file_actions_adddup2(&actions, pipes[1], 1) == 0
            && posix_spawn_file_actions_addopen(&actions, 2, "/dev/null", O_WRONLY, 0) == 0
        let pointers = ([path] + arguments).map { strdup($0) } + [nil]
        defer { for pointer in pointers { free(pointer) } }
        let environment = ProcessInfo.processInfo.environment.map { strdup($0.key + "=" + $0.value) } + [nil]
        defer { for pointer in environment { free(pointer) } }
        var spawned: pid_t = 0
        let status = configured ? pointers.withUnsafeBufferPointer { argv in
            environment.withUnsafeBufferPointer { env in posix_spawn(&spawned, path, &actions, &attributes, argv.baseAddress!, env.baseAddress!) }
        } : EINVAL
        Darwin.close(pipes[1])
        guard status == 0, (try? admit(spawned, identifier: identifier)) != nil else { return nil }
        let child = spawned
        let timers = timeout.map { limit in
            [(limit, SIGTERM), (limit + 5, SIGKILL)].map { delay, signal -> DispatchSourceTimer in
                let source = DispatchSource.makeTimerSource(queue: .global())
                source.schedule(deadline: .now() + delay)
                source.setEventHandler { kill(child, signal) }
                source.resume()
                return source
            }
        } ?? []
        let output = FileHandle(fileDescriptor: pipes[0], closeOnDealloc: false).readDataToEndOfFile()
        var exited = siginfo_t()
        while waitid(P_PID, id_t(child), &exited, WEXITED | WNOWAIT) < 0 && errno == EINTR {}
        for timer in timers { timer.cancel() }
        var exit: Int32 = 0
        while waitpid(child, &exit, 0) < 0 && errno == EINTR {}
        return exit == 0 ? output : nil
    }

    private static func bundled(_ name: String) -> String {
        Bundle.main.bundleURL.appendingPathComponent("Contents/MacOS/\(name)").path
    }

    #if !LAB
    private static func verified(_ name: String) throws -> String {
        let path = bundled(name)
        try CodeIdentity.verifyFile(path, identifier: name)
        return path
    }
    #endif
}
