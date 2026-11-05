import Foundation
import AppKit
import Darwin
import AppleSession

@main
struct AppleBridgeHelper {
    @MainActor
    static func main() {
        umask(0o077)
        if CommandLine.arguments.count == 2, CommandLine.arguments[1] == "media-facts" {
            exit(MediaFacts.run())
        }
        #if LAB
        if CommandLine.arguments.count == 3, CommandLine.arguments[1] == "lab-tcc-probe" {
            do { try LabTCCProbe.run(CommandLine.arguments[2]) }
            catch { exit(1) }
            return
        }
        #endif
        let app = NSApplication.shared
        app.setActivationPolicy(.accessory)
        do {
            let arguments = Array(CommandLine.arguments.dropFirst())
            let applicationLifecycle = arguments.isEmpty || ["intent-host", "connector-approvals", "setup-snapshot"].contains(arguments[0])
            if !applicationLifecycle { app.finishLaunching() }
            if arguments.first == "intent-host" {
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
