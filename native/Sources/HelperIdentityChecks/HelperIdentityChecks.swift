import AppleSession
import Darwin
import Foundation
import SourceSupport

func launch(socket: String, secret: String, home: String) -> String {
    guard let started = try? NativeSessionLauncher.launch(socket: socket, secretFile: secret, home: home) else { return "refused" }
    defer { started.stop() }
    for _ in 0..<100 {
        if FileManager.default.fileExists(atPath: home + "/session-marker") { return "launched" }
        usleep(50000)
    }
    return "launched-without-marker"
}

func connector(labExecutable: String?) -> String {
    guard let path = try? TrustedChild.bridge(labExecutable: labExecutable) else { return "refused" }
    guard let output = TrustedChild.capture(path, identifier: TrustedChild.bridgeName, arguments: ["connector", "pending"]) else { return "refused" }
    return String(decoding: output, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines) == "[]" ? "ran" : "wrong-output"
}

func admit(binary: String, marker: String) -> String {
    var attributes: posix_spawnattr_t?
    guard posix_spawnattr_init(&attributes) == 0 else { return "error" }
    defer { posix_spawnattr_destroy(&attributes) }
    guard posix_spawnattr_setflags(&attributes, Int16(TrustedChild.spawnFlags)) == 0 else { return "error" }
    var spawned: pid_t = 0
    let arguments: [String] = [binary, "marker", marker]
    let pointers = arguments.map { strdup($0) } + [nil]
    defer { for pointer in pointers { free(pointer) } }
    let status = pointers.withUnsafeBufferPointer { argv in posix_spawn(&spawned, binary, nil, &attributes, argv.baseAddress!, nil) }
    guard status == 0 else { return "error" }
    guard (try? TrustedChild.admit(spawned, identifier: TrustedChild.bridgeName)) != nil else { return "refused" }
    var exitStatus: Int32 = 0
    while waitpid(spawned, &exitStatus, 0) < 0 && errno == EINTR {}
    return "admitted"
}

@main
enum HelperIdentityChecks {
    static func main() {
        let arguments = Array(CommandLine.arguments.dropFirst())
        switch arguments.first {
        case "launch" where arguments.count == 4:
            print(launch(socket: arguments[1], secret: arguments[2], home: arguments[3]))
        case "connector" where arguments.count <= 2:
            print(connector(labExecutable: arguments.dropFirst().first))
        case "admit" where arguments.count == 3:
            print(admit(binary: arguments[1], marker: arguments[2]))
        default:
            exit(2)
        }
    }
}
