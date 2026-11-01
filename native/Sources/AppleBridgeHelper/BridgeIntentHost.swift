import AppKit
import AppIntents
import SwiftUI
import CoreServices

@MainActor
enum BridgeIntentHost {
    static func runApplication(_ controller: PackagedApp? = nil) {
        BridgeIntentApplicationDelegate.controller = controller
        BridgeIntentApplication.main()
    }

    static func run(_ arguments: [String]) {
        guard arguments.count == 3, let parent = Int32(arguments[1]), parent > 1 else { return }
        let report = URL(fileURLWithPath: arguments[2])
        guard LSRegisterURL(Bundle.main.bundleURL as CFURL, true) == noErr else { return }
        BridgeShortcuts.updateAppShortcutParameters()
        try? Data(String(getpid()).utf8).write(to: report, options: .atomic)
        let ready = Task { @MainActor in
            try? Data().write(to: report.appendingPathExtension("ready"), options: .atomic)
        }
        defer { ready.cancel() }
        let lifetime = DispatchSource.makeProcessSource(identifier: parent, eventMask: .exit, queue: .main)
        lifetime.setEventHandler { NSApplication.shared.terminate(nil) }
        lifetime.resume()
        defer { lifetime.cancel() }
        if kill(parent, 0) == 0 { runApplication() }
    }
}

private struct BridgeIntentApplication: App {
    @NSApplicationDelegateAdaptor(BridgeIntentApplicationDelegate.self) private var delegate

    var body: some Scene {
        Settings { EmptyView() }
    }
}

@MainActor
private final class BridgeIntentApplicationDelegate: NSObject, NSApplicationDelegate {
    static var controller: PackagedApp?

    func applicationDidFinishLaunching(_ notification: Notification) {
        Self.controller?.show()
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        Self.controller?.applicationShouldHandleReopen(sender, hasVisibleWindows: flag) ?? false
    }
}
