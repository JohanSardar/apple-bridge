import Foundation
import AppKit
import Darwin
import AppleSession

@main
struct AppleBridgeHelper {
    @MainActor
    static func main() {
        umask(0o077)
        let app = NSApplication.shared
        app.setActivationPolicy(.accessory)
        app.finishLaunching()
        do {
            try HelperCommand.run(Array(CommandLine.arguments.dropFirst()))
        } catch {
            exit(1)
        }
    }
}
