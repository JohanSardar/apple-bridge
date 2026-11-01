import Foundation
import AppKit
import Darwin
import AppleSession
import ShortcutsSource

@main
struct AppleBridgeHelper {
    @MainActor
    static func main() {
        umask(0o077)
        let app = NSApplication.shared
        app.setActivationPolicy(.accessory)
        do {
            let arguments = Array(CommandLine.arguments.dropFirst())
            let applicationLifecycle = arguments.isEmpty || ["intent-host", "connector-approvals", "setup-snapshot"].contains(arguments[0])
            if !applicationLifecycle { app.finishLaunching() }
            if arguments.first == "clock-timers", arguments.count == 3 {
                let request = try Data(contentsOf: URL(fileURLWithPath: arguments[1]))
                let result = try ClockTimers.exchange(request)
                try result.write(to: URL(fileURLWithPath: arguments[2]), options: .atomic)
            } else if arguments.first == "intent-host" {
                BridgeIntentHost.run(arguments)
            } else if arguments.isEmpty || ["install", "uninstall", "service-status", "connector-approvals", "setup-snapshot"].contains(arguments[0]) {
                try PackagedApp.run(arguments)
            } else {
                try HelperCommand.run(arguments)
            }
        } catch {
            FileHandle.standardError.write(Data("\(error)\n".utf8))
            exit(1)
        }
    }
}
