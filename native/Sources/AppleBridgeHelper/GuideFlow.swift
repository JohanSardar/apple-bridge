import Foundation

struct GuideStep: Codable, Equatable, Sendable {
    enum Kind: String, Codable, Sendable { case prompt, drag }
    let id: String
    let title: String
    let reason: String
    let kind: Kind
    let pane: String
    let symbol: String

    var settingsURL: URL? { pane.isEmpty ? nil : URL(string: "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?" + pane) }

    static let catalog: [Self] = [
        .init(id: "contacts", title: "Contacts", reason: "So your AI sees names, not phone numbers.", kind: .prompt, pane: "", symbol: "person.crop.circle.fill"),
        .init(id: "calendar", title: "Calendar", reason: "So your AI can read and plan your events.", kind: .prompt, pane: "", symbol: "calendar"),
        .init(id: "reminders", title: "Reminders", reason: "So your AI can read and add to your lists.", kind: .prompt, pane: "", symbol: "checklist"),
        .init(id: "photos", title: "Photos", reason: "So your AI can find a photo when you ask.", kind: .prompt, pane: "", symbol: "photo.on.rectangle.angled"),
        .init(id: "camera", title: "Camera", reason: "So your AI can take a photo when you ask.", kind: .prompt, pane: "", symbol: "camera.fill"),
        .init(id: "microphone", title: "Microphone", reason: "So your AI can record audio when you ask.", kind: .prompt, pane: "", symbol: "mic.fill"),
        .init(id: "location", title: "Location", reason: "So your AI can answer where-am-I questions.", kind: .prompt, pane: "", symbol: "location.fill"),
        .init(id: "full_disk_access", title: "Full Disk Access", reason: "So your AI can read Messages, Notes and Mail.", kind: .drag, pane: "Privacy_AllFiles", symbol: "internaldrive.fill"),
        .init(id: "screen_recording", title: "Screen Recording", reason: "So your AI can take a screenshot when you ask.", kind: .drag, pane: "Privacy_ScreenCapture", symbol: "rectangle.dashed.badge.record")
    ]

    static let standard = ["contacts", "calendar", "reminders", "photos", "full_disk_access", "screen_recording"]

    static func steps(_ ids: [String]) -> [Self] {
        let wanted = ids.isEmpty ? standard : ids
        return catalog.filter { wanted.contains($0.id) }.sorted { first, second in
            first.kind != second.kind ? first.kind == .prompt : (wanted.firstIndex(of: first.id) ?? 0) < (wanted.firstIndex(of: second.id) ?? 0)
        }
    }
}

struct GuideFlow: Codable, Equatable, Sendable {
    enum Moment: String, Codable, Sendable { case asking, waitingForDrop, confirming, cancelledPassword, allowed, declined, finished }

    var steps: [GuideStep]
    var index = 0
    var moment = Moment.asking
    var allowed: [String] = []
    var skipped: [String] = []
    var declined: [String] = []
    var report = ""
    var started = Date()

    init(steps: [GuideStep], report: String = "", started: Date = Date()) {
        self.steps = steps
        self.report = report
        self.started = started
        moment = steps.isEmpty ? .finished : steps[0].kind == .drag ? .waitingForDrop : .asking
    }

    var current: GuideStep? { index < steps.count ? steps[index] : nil }
    var finished: Bool { current == nil }
    var position: String { "STEP \(min(index + 1, steps.count)) OF \(steps.count)" }

    mutating func dropAlreadyAllowed(_ granted: Set<String>) {
        let already = steps.filter { granted.contains($0.id) && !allowed.contains($0.id) }.map(\.id)
        allowed += already.filter { !allowed.contains($0) }
        steps.removeAll { already.contains($0.id) }
        let handled = Set(allowed + skipped + declined)
        index = steps.firstIndex { !handled.contains($0.id) } ?? steps.count
        settle()
    }

    mutating func dropped() {
        guard current?.kind == .drag, moment != .allowed else { return }
        moment = .confirming
    }

    mutating func passwordClosed(granted: Bool) {
        guard current?.kind == .drag, moment == .confirming else { return }
        if granted { allow() } else { moment = .cancelledPassword }
    }

    mutating func allow() {
        guard let step = current, moment != .allowed else { return }
        if !allowed.contains(step.id) { allowed.append(step.id) }
        moment = .allowed
    }

    mutating func decline() {
        guard let step = current else { return }
        if !declined.contains(step.id) { declined.append(step.id) }
        moment = .declined
    }

    mutating func skip() {
        guard let step = current else { return }
        if !skipped.contains(step.id) { skipped.append(step.id) }
        index += 1
        settle()
    }

    mutating func next() {
        guard current != nil else { return }
        index += 1
        settle()
    }

    private mutating func settle() {
        guard let step = current else { moment = .finished; return }
        moment = step.kind == .drag ? .waitingForDrop : .asking
    }

    var summary: [String: String] {
        ["allowed": allowed.joined(separator: ","), "skipped": skipped.joined(separator: ","), "declined": declined.joined(separator: ","), "outcome": finished ? "finished" : "stopped"]
    }

    static let resumeWindow: TimeInterval = 1800

    static func resumable(_ data: Data?, now: Date = Date()) -> Self? {
        guard let data, let flow = try? JSONDecoder().decode(Self.self, from: data), !flow.finished, now.timeIntervalSince(flow.started) < resumeWindow else { return nil }
        return flow
    }
}
