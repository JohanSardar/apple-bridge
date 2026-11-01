import Foundation
import SourceSupport

public enum NotesRichEdit {
    public static func validate(_ before: JSON, html: String) throws {
        guard before["content_status"].string == "complete", before["block_status"].string == "complete",
              before["round_trip_status"].string == "unproven", before["attachments"].array.isEmpty,
              !before["locked"].bool, !before["read_only"].bool, !before["recently_deleted"].bool,
              !html.isEmpty, html.utf8.count <= 1_000_000 else { throw SourceFailure.unsupported }
        let original = before["html"].string
        guard !["data-notes-read-only", "<table", "<ul", "<ol", "<pre", "<blockquote"].contains(where: { original.contains($0) }) else { throw SourceFailure.unsupported }
        let pattern = try NSRegularExpression(pattern: "<[^>]*>|[^<]+")
        func structure(_ value: String) -> [String]? {
            let matches = pattern.matches(in: value, range: NSRange(value.startIndex..., in: value))
            guard matches.reduce(0, { $0 + $1.range.length }) == value.utf16.count else { return nil }
            return pattern.matches(in: value, range: NSRange(value.startIndex..., in: value)).compactMap { match in
                guard let range = Range(match.range, in: value) else { return nil }
                let token = String(value[range])
                return token.hasPrefix("<") ? token : "text"
            }
        }
        guard !original.isEmpty, let originalStructure = structure(original), originalStructure == structure(html),
              !html.contains("\u{0}") else { throw SourceFailure.unsupported }
    }
}
