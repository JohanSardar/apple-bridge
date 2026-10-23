import Foundation
import SourceSupport
import Decoders
import UniformTypeIdentifiers
import CryptoKit

struct NotesAttachment: Sendable {
    let id: String
    let uti: String
    let alternateText: String
    let name: String
    let file: URL?
    let stamp: String
    let payload: Data?
    var metadata: JSON {
        let inline = uti == "com.apple.notes.inlinetextattachment.hashtag"
        let local = file != nil || payload != nil
        var value: [String: JSON] = ["id": .string(id), "uti": .string(uti), "name": .string(name), "state": .string(inline ? "inline" : local ? "local" : "unavailable")]
        if !local && !inline { value["reason"] = .string("attachment_bytes_unavailable") }
        if !alternateText.isEmpty { value["alternate_text"] = .string(alternateText) }
        if let mime = UTType(uti)?.preferredMIMEType { value["mime"] = .string(mime) }
        return .object(value)
    }
    static func belongs(_ row: [String: JSON], note: Int, query: (String, [String]) throws -> [[String: JSON]]) -> Bool {
        var current = row, seen = Set<Int>()
        for _ in 0..<16 {
            if ["ZNOTE", "ZNOTE1"].contains(where: { Int(current[$0]?.number ?? 0) == note }) { return true }
            let parent = Int(current["ZPARENTATTACHMENT"]?.number ?? 0)
            guard parent > 0, seen.insert(parent).inserted,
                  let next = try? query("SELECT * FROM ZICCLOUDSYNCINGOBJECT WHERE Z_PK=?", [String(parent)]).first else { return false }
            current = next
        }
        return false
    }
    static func unsupported(_ block: NotesBlock) -> Bool {
        block.unsupported != nil || block.children.contains(where: unsupported) || (block.rows ?? []).flatMap { $0 }.flatMap { $0.blocks }.contains(where: unsupported)
    }
    init(row: [String: JSON], account: String, root: URL, query: (String, [String]) throws -> [[String: JSON]]) throws {
        id = row["ZIDENTIFIER"]?.string ?? ""
        uti = row["ZTYPEUTI1"]?.string ?? row["ZTYPEUTI"]?.string ?? ""
        payload = uti == "com.apple.notes.table" ? ["ZMERGEABLEDATA1", "ZMERGEABLEDATA"].compactMap { row[$0]?.string }.compactMap { Data(base64Encoded: $0) }.first : nil
        alternateText = row["ZALTTEXT"]?.string ?? ""
        var candidates: [URL] = []
        let roots = Self.component(account) ? [root.appendingPathComponent("Accounts").appendingPathComponent(account), root] : [root]
        let mediaID = Int(row["ZMEDIA"]?.number ?? 0)
        let media = mediaID > 0 ? try query("SELECT * FROM ZICCLOUDSYNCINGOBJECT WHERE Z_PK=?", [String(mediaID)]).first ?? [:] : [:]
        let filename = media["ZFILENAME"]?.string ?? row["ZFILENAME"]?.string ?? ""
        name = Self.component(filename) ? filename : (Self.component(id) ? id : "Attachment")
        let identifier = media["ZIDENTIFIER"]?.string ?? ""
        if Self.component(identifier), Self.component(filename) {
            let generation = ["ZGENERATION", "ZGENERATION1", "ZFALLBACKIMAGEGENERATION", "ZFALLBACKPDFGENERATION", "ZPAPERBUNDLEGENERATION"].compactMap { media[$0]?.string }.first(where: Self.component)
            for base in roots {
                let directory = base.appendingPathComponent("Media").appendingPathComponent(identifier)
                if let generation { candidates.append(directory.appendingPathComponent(generation).appendingPathComponent(filename)) }
                candidates.append(directory.appendingPathComponent(filename))
            }
        }
        if Self.component(id) {
            for base in roots {
                let directory = base.appendingPathComponent("FallbackImages")
                if let generation = row["ZFALLBACKIMAGEGENERATION"]?.string, Self.component(generation) {
                    for ext in ["png", "jpeg", "jpg"] { candidates.append(directory.appendingPathComponent(id).appendingPathComponent(generation).appendingPathComponent("FallbackImage." + ext)) }
                }
                for ext in ["png", "jpeg", "jpg"] { candidates.append(directory.appendingPathComponent(id + "." + ext)) }
            }
        }
        let prefix = root.resolvingSymlinksInPath().path + "/"
        let available = candidates.first { candidate in
            guard candidate.resolvingSymlinksInPath().path.hasPrefix(prefix),
                  let info = try? candidate.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey]) else { return false }
            return info.isRegularFile == true && info.isSymbolicLink != true
        }
        file = available
        if let available, let attrs = try? FileManager.default.attributesOfItem(atPath: available.path) {
            stamp = [attrs[.systemFileNumber], attrs[.size], attrs[.modificationDate]].map { String(describing: $0 ?? "") }.joined(separator: ":")
        } else if let payload { stamp = SHA256.hash(data: payload).map { String(format: "%02x", $0) }.joined() }
        else { stamp = "unavailable" }
    }
    private static func component(_ value: String) -> Bool {
        !value.isEmpty && value != "." && value != ".." && !value.contains("/") && !value.contains("\0")
    }
}
