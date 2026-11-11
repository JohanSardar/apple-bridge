import AppKit

enum KeepAwake {
    @MainActor static func set(_ on: Bool) -> Bool {
        let reason = on
            ? "Apple Bridge wants to keep this Mac awake with the lid closed, so your agents can still reach it."
            : "Apple Bridge wants to let this Mac sleep with the lid closed again."
        NSApplication.shared.activate()
        var error: NSDictionary?
        NSAppleScript(source: "do shell script \"/usr/bin/pmset -a disablesleep \(on ? 1 : 0)\" with prompt \"\(reason)\" with administrator privileges")?.executeAndReturnError(&error)
        return error == nil
    }
}
