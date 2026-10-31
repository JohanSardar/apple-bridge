import Foundation

struct SetupPermission: Identifiable, Sendable {
    let id: String
    let title: String
    let why: String
    let pane: String
    var bundle: String? = nil
    var settingsURL: URL { URL(string: "x-apple.systempreferences:" + pane)! }
    var fix: String { "Other enabled features still work. Open Settings, allow Apple Bridge for \(title), then return here. You can also skip for now." }

    static let all: [Self] = [
        .init(id: "full_disk_access", title: "Full Disk Access", why: "Read Messages, Notes and local files.", pane: "com.apple.preference.security?Privacy_AllFiles"),
        .init(id: "accessibility", title: "Accessibility", why: "Use app controls for actions you approve.", pane: "com.apple.preference.security?Privacy_Accessibility"),
        .init(id: "contacts", title: "Contacts", why: "Find people and update contacts.", pane: "com.apple.preference.security?Privacy_Contacts"),
        .init(id: "calendar", title: "Calendar", why: "Read and update your events.", pane: "com.apple.preference.security?Privacy_Calendars"),
        .init(id: "reminders", title: "Reminders", why: "Read and update your lists.", pane: "com.apple.preference.security?Privacy_Reminders"),
        .init(id: "photos", title: "Photos", why: "Find, import and share photos.", pane: "com.apple.preference.security?Privacy_Photos"),
        .init(id: "camera", title: "Camera", why: "Take a photo when you ask.", pane: "com.apple.preference.security?Privacy_Camera"),
        .init(id: "microphone", title: "Microphone", why: "Record audio when you ask.", pane: "com.apple.preference.security?Privacy_Microphone"),
        .init(id: "screen_recording", title: "Screen Recording", why: "Capture your screen when you ask.", pane: "com.apple.preference.security?Privacy_ScreenCapture"),
        .init(id: "location", title: "Location", why: "Answer nearby and location questions.", pane: "com.apple.preference.security?Privacy_LocationServices"),
        .init(id: "notifications", title: "Notifications", why: "Show alerts and approval requests.", pane: "com.apple.Notifications-Settings.extension"),
        .init(id: "focus", title: "Focus", why: "Read and share your Focus status.", pane: "com.apple.Focus-Settings.extension"),
        .init(id: "speech_recognition", title: "Speech", why: "Turn recordings into text.", pane: "com.apple.preference.security?Privacy_SpeechRecognition"),
        .init(id: "bluetooth", title: "Bluetooth", why: "Read nearby Bluetooth device status.", pane: "com.apple.preference.security?Privacy_Bluetooth"),
        .init(id: "clipboard", title: "Clipboard", why: "Read copied text when you ask.", pane: "com.apple.preference.security?Privacy_Pasteboard")
    ] + [
        ("messages", "Messages", "com.apple.MobileSMS", "Send messages you approve."),
        ("system_events", "System Events", "com.apple.systemevents", "Use app controls and Mac settings."),
        ("reminders", "Reminders", "com.apple.reminders", "Use extra list controls."),
        ("mail", "Mail", "com.apple.mail", "Send and organize mail."),
        ("music", "Music", "com.apple.Music", "Control playback and playlists."),
        ("safari", "Safari", "com.apple.Safari", "Open tabs and Reading List items."),
        ("shortcuts_events", "Shortcuts Events", "com.apple.shortcuts.events", "Run shortcuts you choose."),
        ("calendar", "Calendar", "com.apple.iCal", "Use extra event controls."),
        ("notes", "Notes", "com.apple.Notes", "Create and edit notes.")
    ].map { .init(id: "automation_" + $0.0, title: "Automation · " + $0.1, why: $0.3, pane: "com.apple.preference.security?Privacy_Automation", bundle: $0.2) }
}

struct SetupState: Codable {
    var step = 0
    var skipped: Set<String> = []
    var completed: Set<Int> = []
    var phone = false
    var connectors: Set<String> = []
    static let titles = ["Welcome", "Permissions", "Pair iPhone", "Connect AI", "Done"]
    mutating func advance() {
        completed.insert(step)
        step = min(step + 1, 4)
    }
    mutating func back() { step = max(0, step - 1) }
    func label(_ id: String, status: String?) -> String {
        if ["granted", "authorized"].contains(status ?? "") { return "Allowed" }
        if skipped.contains(id) { return "Skipped" }
        switch status {
        case "denied", "restricted": return "Needs permission"
        case "limited": return "Limited access"
        case "not_determined": return "Not requested"
        case "target_not_running": return "Open the target app to check"
        default: return "Not confirmed"
        }
    }
}
