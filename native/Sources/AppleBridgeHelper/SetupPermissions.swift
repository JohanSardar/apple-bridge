import AppKit
import AppleSession
import Contacts
import EventKit
import Carbon
import ApplicationServices

enum SetupProbe {
    static let argument = "setup-probe"

    static func fullDiskAccess() -> String {
        do {
            let file = try FileHandle(forReadingFrom: FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/com.apple.TCC/TCC.db"))
            try file.close()
            return "granted"
        } catch {
            let failure = error as NSError
            let underlying = (failure.userInfo[NSUnderlyingErrorKey] as? NSError)?.code
            return failure.code == NSFileReadNoPermissionError || underlying == 1 || underlying == 13 ? "denied" : "unknown"
        }
    }

    static func run() -> Int32 {
        let report = ["full_disk_access": fullDiskAccess(), "screen_recording": CGPreflightScreenCaptureAccess() ? "granted" : "denied"]
        guard let data = try? JSONSerialization.data(withJSONObject: report) else { return 1 }
        FileHandle.standardOutput.write(data)
        return 0
    }

    static func fresh() async -> [String: String] {
        guard let executable = Bundle.main.executablePath else { return [:] }
        return await Task.detached { spawn(executable) }.value
    }

    private typealias Disclaim = @convention(c) (UnsafeMutablePointer<posix_spawnattr_t?>, Int32) -> Int32

    private static func spawn(_ executable: String) -> [String: String] {
        var pipes: [Int32] = [-1, -1]
        guard pipe(&pipes) == 0 else { return [:] }
        defer { close(pipes[0]) }
        var actions: posix_spawn_file_actions_t?
        var attributes: posix_spawnattr_t?
        posix_spawn_file_actions_init(&actions)
        posix_spawnattr_init(&attributes)
        defer { posix_spawn_file_actions_destroy(&actions); posix_spawnattr_destroy(&attributes) }
        posix_spawn_file_actions_adddup2(&actions, pipes[1], 1)
        posix_spawn_file_actions_addopen(&actions, 2, "/dev/null", O_WRONLY, 0)
        posix_spawnattr_setflags(&attributes, Int16(POSIX_SPAWN_CLOEXEC_DEFAULT))
        if let symbol = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "responsibility_spawnattrs_setdisclaim") {
            _ = unsafeBitCast(symbol, to: Disclaim.self)(&attributes, 1)
        }
        let argv = [strdup(executable), strdup(argument), nil]
        let environment = ProcessInfo.processInfo.environment.map { strdup($0.key + "=" + $0.value) } + [nil]
        defer { (argv + environment).forEach { free($0) } }
        var pid: pid_t = 0
        let started = posix_spawn(&pid, executable, &actions, &attributes, argv, environment)
        close(pipes[1])
        guard started == 0 else { return [:] }
        let data = FileHandle(fileDescriptor: pipes[0], closeOnDealloc: false).readDataToEndOfFile()
        var status: Int32 = 0
        while waitpid(pid, &status, 0) < 0 && errno == EINTR {}
        return (try? JSONDecoder().decode([String: String].self, from: data)) ?? [:]
    }
}

@MainActor
final class SetupPermissions {
    func status() async -> [String: String] {
        var result = await NewPermissions.status()
        result["accessibility"] = AXIsProcessTrusted() ? "granted" : "denied"
        let contact = CNContactStore.authorizationStatus(for: .contacts)
        result["contacts"] = contact == .authorized ? "granted" : contact == .notDetermined ? "not_determined" : "denied"
        for (name, type) in [("calendar", EKEntityType.event), ("reminders", EKEntityType.reminder)] {
            let state = EKEventStore.authorizationStatus(for: type)
            result[name] = state == .fullAccess ? "granted" : state == .notDetermined ? "not_determined" : "denied"
        }
        let fresh = await SetupProbe.fresh()
        result["full_disk_access"] = fresh["full_disk_access"] ?? SetupProbe.fullDiskAccess()
        if fresh["screen_recording"] == "granted" { result["screen_recording"] = "granted" }
        for permission in SetupPermission.all {
            guard let bundle = permission.bundle else { continue }
            let code = await Task.detached {
                let target = NSAppleEventDescriptor(bundleIdentifier: bundle)
                return AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), false)
            }.value
            result[permission.id] = code == noErr ? "granted" : code == -1744 ? "not_determined" : code == -1743 ? "denied" : code == -600 ? "target_not_running" : "unknown"
        }
        return result
    }

    func request(_ permission: SetupPermission) async throws {
        if let bundle = permission.bundle {
            var launched: NSRunningApplication?
            if NSRunningApplication.runningApplications(withBundleIdentifier: bundle).isEmpty,
               let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundle) {
                let configuration = NSWorkspace.OpenConfiguration()
                configuration.activates = false
                configuration.hides = true
                configuration.addsToRecentItems = false
                launched = try await NSWorkspace.shared.openApplication(at: url, configuration: configuration)
            }
            let code = await Task.detached {
                let target = NSAppleEventDescriptor(bundleIdentifier: bundle)
                var code = AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), true)
                for _ in 0..<20 where code == -600 {
                    try? await Task.sleep(for: .milliseconds(250))
                    code = AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), true)
                }
                return code
            }.value
            launched?.terminate()
            if code != noErr { throw CocoaError(.userCancelled) }
            return
        }
        switch permission.id {
        case "contacts": _ = try await CNContactStore().requestAccess(for: .contacts)
        case "calendar": _ = try await EKEventStore().requestFullAccessToEvents()
        case "reminders": _ = try await EKEventStore().requestFullAccessToReminders()
        default: try await NewPermissions.requestNative(permission.id)
        }
    }
}
