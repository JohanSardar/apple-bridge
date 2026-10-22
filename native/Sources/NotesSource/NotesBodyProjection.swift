import Foundation
import CryptoKit
import SourceSupport

@MainActor
enum NotesBodyProjection {
    static func references(_ data: [String: JSON]) -> [JSON] {
        var result: [JSON] = []
        for attachment in data["attachments", default: .array([])].array {
            if attachment["blob"] != .null { result.append(attachment["blob"]) }
            result.append(contentsOf: attachment["derivatives"].object.values)
        }
        for key in ["raw_body", "body_blob", "thumbnail"] {
            if let value = data[key] { result.append(value) }
        }
        return result
    }
    private static func size(_ data: [String: JSON]) throws -> Int {
        try JSONEncoder().encode(JSON.object(data)).count + Set(references(data).map { $0["digest"].string }).count * 160 + 4096
    }
    static func fit(_ original: [String: JSON], assets: NotesAssets) async throws -> [String: JSON] {
        guard try size(original) >= 224000 else { return original }
        var data = original
        if original["locked"]?.bool == true {
            data["title"] = .string(String(String.UnicodeScalarView(original["title", default: .string("")].string.unicodeScalars.prefix(160))))
            data["title_status"] = .string("metadata_capacity")
            return data
        }
        let encoder = JSONEncoder(); encoder.outputFormatting = [.sortedKeys]
        let body = try encoder.encode(JSON.object(["plain_text": original["plain_text"] ?? .string(""), "html": original["html"] ?? .string(""), "blocks": original["blocks"] ?? .array([])]))
        let lines = original["plain_text", default: .string("")].string.components(separatedBy: "\n")
        let content = lines.first?.trimmingCharacters(in: .whitespacesAndNewlines) == original["title", default: .string("")].string.trimmingCharacters(in: .whitespacesAndNewlines) ? Array(lines.dropFirst()) : lines
        let preview = content.map { $0.replacingOccurrences(of: "\u{fffc}", with: "").split(whereSeparator: \.isWhitespace).joined(separator: " ") }.first(where: { !$0.isEmpty }) ?? ""
        data["preview"] = .string(String(String.UnicodeScalarView(preview.unicodeScalars.prefix(160))))
        data.removeValue(forKey: "html"); data.removeValue(forKey: "blocks")
        data["editable"] = .string("none")
        data["write_limit_reason"] = .string("external_body_write_unqualified")
        do { data["body_blob"] = try await assets.stage(body); data["body_status"] = .string("external") }
        catch {
            try Task.checkCancellation()
            let digest = SHA256.hash(data: body).map { String(format: "%02x", $0) }.joined()
            data["body_blob"] = .object(["digest": .string(digest), "size": .number(Double(body.count)), "state": .string("missing")])
            data["body_status"] = .string("unavailable")
            data["content_status"] = .string("offloaded")
        }
        if try size(data) >= 224000 { data.removeValue(forKey: "plain_text"); data["search_status"] = .string("metadata_only") }
        if try size(data) >= 224000 {
            let thumb = data["attachments", default: .array([])].array.compactMap { $0["derivatives"]["thumb"] == .null ? nil : $0["derivatives"]["thumb"] }.first
            data["thumbnail"] = thumb
            data["attachments"] = .array([])
            data["attachment_status"] = .string("metadata_capacity")
            data["block_status"] = .string("partial")
        }
        if try size(data) >= 224000 {
            data.removeValue(forKey: "tags")
            data["tags_status"] = .string("metadata_capacity")
            data["title"] = .string(String(String.UnicodeScalarView(original["title", default: .string("")].string.unicodeScalars.prefix(160))))
            data["title_status"] = .string("metadata_capacity")
        }
        guard try size(data) < 224000 else { throw SourceFailure.invalid }
        return data
    }
}
