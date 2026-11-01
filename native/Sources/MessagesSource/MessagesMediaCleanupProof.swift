import Foundation
import SourceSupport

public struct MessagesMediaCleanupProof: Sendable {
    public let baseline: Int64
    public let names: Set<String>
    private let displayNames: [String: Set<String>]

    public init(registration: JSON, tag: String) throws {
        guard tag.range(of: #"^\[ab-test [A-Za-z0-9_-]+\]$"#, options: .regularExpression) != nil else { throw SourceFailure.invalid }
        let run = String(tag.dropFirst(9).dropLast())
        switch registration["baseline_max_row_id"] {
        case .integer, .number: break
        default: throw SourceFailure.invalid
        }
        guard ["WEB-MEDIA-VARIANTS", "MSG-SELFTEST", "WEB-UPLOAD-NATIVE-STAGING"].contains(registration["scenario"].string), registration["run_id"].string == run,
              let baseline = Int64(exactly: registration["baseline_max_row_id"].number), baseline >= 0 else { throw SourceFailure.invalid }
        let files = registration["media_attachment_names"].array
        guard !files.isEmpty else { throw SourceFailure.invalid }
        var names: Set<String> = []
        var displayNames: [String: Set<String>] = [:]
        for (index, file) in files.enumerated() {
            let name = file.string
            guard name.hasPrefix("ab-" + run + "."), name == URL(fileURLWithPath: name).lastPathComponent, !name.contains("%"), !name.contains("\n") else { throw SourceFailure.invalid }
            names.insert(name)
            let staged = String(index) + "-" + name
            names.insert(staged)
            displayNames[name] = [name, staged]
            displayNames[staged] = [name, staged]
        }
        self.baseline = baseline
        self.names = names
        self.displayNames = displayNames
    }

    public func authorizes(rowID: Int64, attachments: [String], peers: [String], own: Set<String>) -> Bool {
        rowID > baseline && !peers.isEmpty && peers.allSatisfy { own.contains(MessagesProbes.normalize($0)) } && !attachments.isEmpty && attachments.allSatisfy { names.contains(URL(fileURLWithPath: $0).lastPathComponent) }
    }

    public func targetNames(_ attachments: [String]) -> [String] {
        Set(attachments.flatMap { Array(displayNames[URL(fileURLWithPath: $0).lastPathComponent] ?? []) }).sorted()
    }

    public static func matches(_ value: String, names: Set<String>) -> Bool {
        guard !value.isEmpty else { return false }
        let decoded = value.removingPercentEncoding ?? value
        if names.contains(decoded) { return true }
        let fields = decoded.components(separatedBy: ", ")
        if fields.count >= 3, fields.dropFirst().contains(where: names.contains) { return true }
        if let url = URL(string: value), url.isFileURL { return names.contains(url.lastPathComponent) }
        return decoded.hasPrefix("/") && names.contains(URL(fileURLWithPath: decoded).lastPathComponent)
    }

    public static func direction(_ description: String, names: Set<String>) -> Bool? {
        let fields = description.components(separatedBy: ", ")
        guard fields.count >= 2, !fields[0].isEmpty else { return nil }
        if ["Your iMessage", "Your SMS", "Your RCS"].contains(fields[0]) { return true }
        guard !fields[0].hasPrefix("Your ") else { return nil }
        return false
    }

    public static func targetIndex(_ candidates: [JSON], names: Set<String>, outgoing: Bool, allowSelfEcho: Bool = false, ownedCopies: Int = 1) -> Int? {
        let targets = candidates.enumerated().filter { _, candidate in
            candidate["outgoing"] == .bool(outgoing) && candidate["values"].array.contains { Self.matches($0.string, names: names) }
        }
        if !targets.isEmpty, targets.count <= ownedCopies { return targets.first?.offset }
        guard targets.isEmpty, allowSelfEcho else { return nil }
        let echoes = candidates.enumerated().filter { _, candidate in candidate["values"].array.contains { Self.matches($0.string, names: names) } }
        return !echoes.isEmpty && echoes.count <= ownedCopies ? echoes.first?.offset : nil
    }
}
