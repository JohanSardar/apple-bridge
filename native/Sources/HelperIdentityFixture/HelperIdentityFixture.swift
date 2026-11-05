import Darwin
import Foundation

func touch(_ path: String) {
    FileManager.default.createFile(atPath: path, contents: Data("ran".utf8), attributes: [.posixPermissions: 0o600])
}

@main
enum HelperIdentityFixture {
    static func main() {
        let arguments = Array(CommandLine.arguments.dropFirst())
        if let index = arguments.firstIndex(of: "--session-home"), index + 1 < arguments.count {
            touch(arguments[index + 1] + "/session-marker")
            sleep(30)
            exit(0)
        }

        switch arguments.first {
        case "marker" where arguments.count == 2:
            touch(arguments[1])
        case "connector":
            print("[]")
        case "listen" where arguments.count == 2:
            let path = arguments[1]
            let server = socket(AF_UNIX, SOCK_STREAM, 0)
            var address = sockaddr_un()
            address.sun_family = sa_family_t(AF_UNIX)
            address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
            let bytes = Array(path.utf8) + [0]
            guard bytes.count <= MemoryLayout.size(ofValue: address.sun_path) else { exit(2) }
            withUnsafeMutableBytes(of: &address.sun_path) { $0.copyBytes(from: bytes) }
            let bound = withUnsafePointer(to: &address) { pointer in
                pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(server, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
            }
            guard bound == 0, chmod(path, 0o600) == 0, listen(server, 8) == 0 else { exit(2) }
            print("listening")
            fflush(stdout)
            while accept(server, nil, nil) >= 0 {}
        default:
            exit(2)
        }
    }
}
