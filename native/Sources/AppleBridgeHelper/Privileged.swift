import AppKit

enum Privileged {
    static let tailscaleSigner = "Developer ID Installer: Tailscale Inc. (W5364U7YZB)"

    @MainActor static func run(_ arguments: [String]) -> Bool {
        var commands: [String] = []
        var install = false
        var awake: Bool?
        var index = 0
        while index < arguments.count {
            switch arguments[index] {
            case "--tailscale-package" where index + 1 < arguments.count:
                let path = arguments[index + 1]
                guard path.hasPrefix("/"), path.range(of: "^[A-Za-z0-9/._-]+$", options: .regularExpression) != nil else { return false }
                commands.append("d=$(/usr/bin/mktemp -d) && /bin/cp '\(path)' \"$d/t.pkg\" && /usr/sbin/pkgutil --check-signature \"$d/t.pkg\" | /usr/bin/grep -qF '\(tailscaleSigner)' && /usr/sbin/installer -pkg \"$d/t.pkg\" -target / >/dev/null; s=$?; /bin/rm -rf \"$d\"; [ $s -eq 0 ]")
                install = true
                index += 2
            case "--keep-awake":
                commands.append("/usr/bin/pmset -a disablesleep 1")
                awake = true
                index += 1
            case "--let-sleep":
                commands.append("/usr/bin/pmset -a disablesleep 0")
                awake = false
                index += 1
            default:
                return false
            }
        }
        guard !commands.isEmpty else { return false }
        let reason: String
        switch (install, awake) {
        case (true, true): reason = "Apple Bridge wants to install Tailscale and keep this Mac awake with the lid closed, so your agents can reach it."
        case (true, _): reason = "Apple Bridge wants to install Tailscale, so your agents can reach this Mac."
        case (false, true): reason = "Apple Bridge wants to keep this Mac awake with the lid closed, so your agents can still reach it."
        default: reason = "Apple Bridge wants to let this Mac sleep with the lid closed again."
        }
        let script = commands.map { "(\($0))" }.joined(separator: " && ")
            .replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "\"", with: "\\\"")
        NSApplication.shared.activate()
        var error: NSDictionary?
        NSAppleScript(source: "do shell script \"\(script)\" with prompt \"\(reason)\" with administrator privileges")?.executeAndReturnError(&error)
        return error == nil
    }
}
