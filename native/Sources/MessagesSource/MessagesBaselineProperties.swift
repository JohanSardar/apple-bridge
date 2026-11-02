import Foundation
import Decoders
import SourceSupport

enum MessagesBaselineProperties {
    static func properties(_ chat: String) throws -> [String: Any] {
        let path = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/chat.db").path
        let row = try MessagesReader(path: path).query("SELECT properties FROM chat WHERE guid=?", [chat]).first
        guard let row else { throw SourceFailure.conflict }
        guard let data = row.blobs["properties"] else { return [:] }
        guard let decoded = MessageArchive.decode(data) else { throw SourceFailure.corrupt }
        return decoded
    }
}
