import Foundation

enum PermissionKind: Sendable {
    case prompt
    case settings
    case automation(String)
}

enum SetupTint: Sendable {
    case blue, green, orange, red, yellow, purple, gray, indigo
}

struct SetupPermission: Identifiable, Sendable {
    let id: String
    let title: String
    let action: String
    let why: String
    let pane: String
    let symbol: String
    let tint: SetupTint
    let kind: PermissionKind
    var prompt = ""
    var button = "Allow"
    var settingsURL: URL { URL(string: "x-apple.systempreferences:com.apple.preference.security?" + pane)! }
    var bundle: String? {
        if case .automation(let bundle) = kind { return bundle }
        return nil
    }
    var usesSettings: Bool {
        if case .settings = kind { return true }
        return false
    }

    static let all: [Self] = [
        .init(id: "full_disk_access", title: "Full Disk Access", action: "Allow Full Disk Access", why: "Apple Bridge reads Messages, Notes, Mail and Safari from their files on this Mac. macOS only allows that with Full Disk Access.", pane: "Privacy_AllFiles", symbol: "internaldrive.fill", tint: .gray, kind: .settings),
        .init(id: "accessibility", title: "Accessibility", action: "Allow Accessibility", why: "Lets Apple Bridge reply to a message, react and edit in Messages, and tidy Notes folders, using the same controls you click.", pane: "Privacy_Accessibility", symbol: "accessibility", tint: .blue, kind: .settings),
        .init(id: "contacts", title: "Contacts", action: "Allow Contacts", why: "Shows names instead of phone numbers, and finds the people you mention.", pane: "Privacy_Contacts", symbol: "person.crop.circle.fill", tint: .orange, kind: .prompt, prompt: "would like to access your Contacts."),
        .init(id: "calendar", title: "Calendar", action: "Allow Calendar", why: "Reads your events and adds new ones when you ask.", pane: "Privacy_Calendars", symbol: "calendar", tint: .red, kind: .prompt, prompt: "would like full access to your Calendar.", button: "Allow Full Access"),
        .init(id: "reminders", title: "Reminders", action: "Allow Reminders", why: "Reads your lists and adds or completes reminders when you ask.", pane: "Privacy_Reminders", symbol: "checklist", tint: .blue, kind: .prompt, prompt: "would like to access your Reminders."),
        .init(id: "photos", title: "Photos", action: "Allow Photos", why: "Finds and shows photos when you ask. Your library is never copied.", pane: "Privacy_Photos", symbol: "photo.on.rectangle.angled", tint: .yellow, kind: .prompt, prompt: "would like to access your Photo Library.", button: "Allow access to all Photos"),
        .init(id: "automation_messages", title: "Messages", action: "Allow sending messages", why: "Lets Apple Bridge send the texts you ask for through Messages.", pane: "Privacy_Automation", symbol: "message.fill", tint: .green, kind: .automation("com.apple.MobileSMS"), prompt: "wants access to control “Messages”."),
        .init(id: "automation_notes", title: "Notes", action: "Allow editing notes", why: "Lets Apple Bridge create and edit notes in Notes.", pane: "Privacy_Automation", symbol: "note.text", tint: .yellow, kind: .automation("com.apple.Notes"), prompt: "wants access to control “Notes”."),
        .init(id: "automation_mail", title: "Mail", action: "Allow sending mail", why: "Lets Apple Bridge draft, send and file mail in Mail.", pane: "Privacy_Automation", symbol: "envelope.fill", tint: .blue, kind: .automation("com.apple.mail"), prompt: "wants access to control “Mail”."),
        .init(id: "automation_safari", title: "Safari", action: "Allow reading Safari tabs", why: "Lets Apple Bridge see the tabs you have open in Safari.", pane: "Privacy_Automation", symbol: "safari.fill", tint: .blue, kind: .automation("com.apple.Safari"), prompt: "wants access to control “Safari”."),
        .init(id: "screen_recording", title: "Screen Recording", action: "Allow Screen Recording", why: "Takes a screenshot or records the screen when you ask.", pane: "Privacy_ScreenCapture", symbol: "rectangle.dashed.badge.record", tint: .purple, kind: .settings),
        .init(id: "camera", title: "Camera", action: "Allow Camera", why: "Takes a photo with the camera when you ask.", pane: "Privacy_Camera", symbol: "camera.fill", tint: .gray, kind: .prompt, prompt: "would like to access the camera."),
        .init(id: "microphone", title: "Microphone", action: "Allow Microphone", why: "Records audio when you ask.", pane: "Privacy_Microphone", symbol: "mic.fill", tint: .orange, kind: .prompt, prompt: "would like to access the microphone."),
        .init(id: "location", title: "Location", action: "Allow Location", why: "Answers questions about where you are right now.", pane: "Privacy_LocationServices", symbol: "location.fill", tint: .blue, kind: .prompt, prompt: "would like to use your current location.")
    ]

    static func named(_ id: String) -> Self? { all.first { $0.id == id } }
}

struct SetupService: Identifiable, Sendable {
    let id: String
    let title: String
    let detail: String
    let app: String?
    let symbol: String
    let permissions: [String]
    let suggested: Bool

    static let all: [Self] = [
        .init(id: "messages", title: "Messages", detail: "Read and send texts", app: "com.apple.MobileSMS", symbol: "message.fill", permissions: ["full_disk_access", "contacts", "automation_messages", "accessibility"], suggested: true),
        .init(id: "photos", title: "Photos", detail: "Find and show photos", app: "com.apple.Photos", symbol: "photo", permissions: ["photos"], suggested: true),
        .init(id: "calendar", title: "Calendar", detail: "Events and free time", app: "com.apple.iCal", symbol: "calendar", permissions: ["calendar"], suggested: true),
        .init(id: "reminders", title: "Reminders", detail: "Lists and to-dos", app: "com.apple.reminders", symbol: "checklist", permissions: ["calendar", "reminders"], suggested: true),
        .init(id: "notes", title: "Notes", detail: "Read and write notes", app: "com.apple.Notes", symbol: "note.text", permissions: ["full_disk_access", "automation_notes"], suggested: true),
        .init(id: "contacts", title: "Contacts", detail: "People and numbers", app: "com.apple.AddressBook", symbol: "person.crop.circle", permissions: ["contacts"], suggested: true),
        .init(id: "mail", title: "Mail", detail: "Read, draft and send", app: "com.apple.mail", symbol: "envelope", permissions: ["full_disk_access", "automation_mail"], suggested: true),
        .init(id: "safari", title: "Safari", detail: "History, bookmarks, tabs", app: "com.apple.Safari", symbol: "safari", permissions: ["full_disk_access", "automation_safari"], suggested: false),
        .init(id: "screen", title: "Screen and Camera", detail: "Screenshots, photos, audio", app: nil, symbol: "camera.viewfinder", permissions: ["screen_recording", "camera", "microphone"], suggested: false),
        .init(id: "location", title: "Location", detail: "Where you are now", app: nil, symbol: "location.fill", permissions: ["location"], suggested: false)
    ]
}

enum SetupStep: String, Codable, CaseIterable, Sendable {
    case welcome, services, permissions, connect, done
}

struct SetupState: Codable {
    var step = SetupStep.welcome
    var services = Set(SetupService.all.filter(\.suggested).map(\.id))
    var skipped: Set<String> = []
    var automated: Set<String> = []
    var current: String?

    var plan: [SetupPermission] {
        let wanted = Set(SetupService.all.filter { services.contains($0.id) }.flatMap(\.permissions))
        return SetupPermission.all.filter { wanted.contains($0.id) }
    }

    static func allowed(_ status: String?) -> Bool {
        ["granted", "authorized", "limited"].contains(status ?? "")
    }

    func allowed(_ id: String, statuses: [String: String]) -> Bool {
        Self.allowed(statuses[id]) || (automated.contains(id) && statuses[id] == "target_not_running")
    }

    func next(after id: String?, statuses: [String: String]) -> SetupPermission? {
        let plan = plan
        let start = id.flatMap { current in plan.firstIndex { $0.id == current }.map { $0 + 1 } } ?? 0
        return plan.dropFirst(start).first { !allowed($0.id, statuses: statuses) && !skipped.contains($0.id) }
    }
}
