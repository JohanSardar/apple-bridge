import Foundation
import AppKit
import Darwin
import AppleSession
import SourceSupport

@main
struct AppleBridgeHelper {
    @MainActor
    static func main() {
        umask(0o077)
        if CommandLine.arguments.count == 2, CommandLine.arguments[1] == "media-facts" {
            guard (try? TrustedChild.verifySessionParent()) != nil else { exit(1) }
            exit(MediaFacts.run())
        }
        if CommandLine.arguments.count == 3, CommandLine.arguments[1] == "permission-probe" {
            print(GuideController.probeHere(CommandLine.arguments[2]))
            exit(0)
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
        if CommandLine.arguments.count == 3, CommandLine.arguments[1] == "keep-awake" {
            exit(KeepAwake.set(CommandLine.arguments[2] == "on") ? 0 : 1)
        }
        do {
            let arguments = Array(CommandLine.arguments.dropFirst())
            let applicationLifecycle = arguments.isEmpty || ["intent-host", "connector-approvals", "permission-guide"].contains(arguments[0])
            if !applicationLifecycle { app.finishLaunching() }
            if arguments.first == "intent-host" {
                BridgeIntentHost.run(arguments)
            } else if arguments.isEmpty || ["install", "uninstall", "service-status", "connector-approvals", "permission-guide"].contains(arguments[0]) {
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
