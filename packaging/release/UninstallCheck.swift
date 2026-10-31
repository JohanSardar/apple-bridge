import Foundation

@main
struct UninstallCheck {
    static func command(_ executable: String, _ arguments: [String]) throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = arguments
        process.standardInput = FileHandle.nullDevice
        process.standardOutput = FileHandle.nullDevice
        try process.run()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else { throw ReleaseError.cleanupFailed }
    }
    static func main() throws {
        let args = Array(CommandLine.arguments.dropFirst())
        guard args.count == 1 else { throw ReleaseError.unsafePath }
        let manager = FileManager.default
        let temp = manager.homeDirectoryForCurrentUser.appendingPathComponent("Library/Caches/ab-test-uninstall-\(UUID().uuidString)").resolvingSymlinksInPath()
        try manager.createDirectory(at: temp, withIntermediateDirectories: true)
        defer { try? manager.removeItem(at: temp) }
        let home = temp.appendingPathComponent("home")
        let app = home.appendingPathComponent("Applications/[ab-test] Apple Bridge.app")
        try manager.createDirectory(at: app.deletingLastPathComponent(), withIntermediateDirectories: true)
        try command("/usr/bin/ditto", [args[0], app.path])
        let infoURL = app.appendingPathComponent("Contents/Info.plist")
        var info = try PropertyListSerialization.propertyList(from: Data(contentsOf: infoURL), format: nil) as! [String: Any]
        let bundle = "dev.applebridge.r1test.uninstall"
        info["CFBundleIdentifier"] = bundle
        try PropertyListSerialization.data(fromPropertyList: info, format: .xml, options: 0).write(to: infoURL)
        try command("/usr/bin/codesign", ["--force", "--deep", "--sign", "-", app.path])
        let register = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
        try command(register, ["-f", app.path])
        defer { try? command(register, ["-u", app.path]) }
        let state = home.appendingPathComponent("Library/Application Support/AppleBridge")
        try manager.createDirectory(at: state, withIntermediateDirectories: true)
        try command(app.appendingPathComponent("Contents/MacOS/bridge").path, ["install-credentials", "--home", state.path])
        precondition(manager.fileExists(atPath: state.appendingPathComponent("Apple Bridge.keychain-db").path))
        for path in ReleaseCleanup.paths(home: home).dropFirst() {
            try manager.createDirectory(at: path.deletingLastPathComponent(), withIntermediateDirectories: true)
            try Data("[ab-test]".utf8).write(to: path)
        }
        try ReleaseCleanup.uninstall(home: home, app: app, unregister: {}, resetPermissions: {
            try ReleaseCleanup.resetPermissions(bundleIdentifier: bundle, command: command)
        }, command: command)
        precondition(!manager.fileExists(atPath: app.path))
        precondition(ReleaseCleanup.paths(home: home).allSatisfy { !manager.fileExists(atPath: $0.path) })
        let files = manager.enumerator(at: home, includingPropertiesForKeys: [.isDirectoryKey])!
        for case let path as URL in files {
            let values = try path.resourceValues(forKeys: [.isDirectoryKey])
            precondition(values.isDirectory == true)
        }
        print("R1-FRESH-HOME-UNINSTALL passed=true")
    }
}
