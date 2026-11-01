import Foundation
import SourceSupport

extension NotesSource {
    static let extrasOperations = ["notes.move", "notes.rename_folder"]
    func extrasCapabilities() -> [String] {
        Self.extrasOperations
    }
}
