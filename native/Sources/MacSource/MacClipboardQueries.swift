import AppKit
import SourceSupport

@MainActor
public enum MacClipboardQueries {
    public static func read(_ args: JSON) throws -> JSON {
        guard args == .object([:]) else { throw SourceFailure.invalid }
        let board = NSPasteboard.general
        guard board.accessBehavior == .alwaysAllow else { throw SourceFailure.permissionMissing }
        guard let text = board.string(forType: .string) else { return .object(["text": .null]) }
        guard text.utf8.count <= 65536 else { throw SourceFailure.invalid }
        return .object(["text": .string(text)])
    }
}
