import Foundation
import Darwin
import SourceSupport

public enum MailBody {
    public static func read(_ args: JSON, root: URL = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Mail/V10")) throws -> JSON {
        let id = args["message_id"].string
        guard args.object.count == 1, !id.isEmpty, id.allSatisfy({ $0.isASCII && $0.isNumber }), id.count < 20 else { throw SourceFailure.invalid }
        _ = try FileManager.default.contentsOfDirectory(atPath: root.path)
        var traversalFailed = false
        guard let files = FileManager.default.enumerator(at: root, includingPropertiesForKeys: [.isRegularFileKey], options: [.skipsHiddenFiles], errorHandler: { _, _ in traversalFailed = true; return false }) else { throw SourceFailure.unavailable }
        var found: URL?
        var count = 0
        for case let file as URL in files {
            try Task.checkCancellation()
            count += 1
            guard count <= 500000 else { throw SourceFailure.unavailable }
            if file.lastPathComponent == id + ".emlx" || file.lastPathComponent == id + ".partial.emlx" {
                guard found == nil else { throw SourceFailure.conflict }
                found = file
            }
        }
        guard !traversalFailed else { throw SourceFailure.unavailable }
        guard let file = found else { return result("offloaded", "", "") }
        let resolved = file.resolvingSymlinksInPath()
        guard resolved.path.hasPrefix(root.resolvingSymlinksInPath().path + "/") else { throw SourceFailure.invalid }
        var metadata = stat()
        guard Darwin.lstat(resolved.path, &metadata) == 0 else { throw SourceFailure.unavailable }
        if metadata.st_flags & UInt32(SF_DATALESS) != 0 { return result("offloaded", "", "") }
        let values = try file.resourceValues(forKeys: [.fileSizeKey, .isRegularFileKey])
        guard values.isRegularFile == true, let size = values.fileSize, size <= 32 << 20 else { throw SourceFailure.unavailable }
        let handle = try FileHandle(forReadingFrom: resolved)
        defer { try? handle.close() }
        let data = try handle.read(upToCount: (32 << 20) + 1) ?? Data()
        guard data.count <= 32 << 20 else { throw SourceFailure.unavailable }
        guard let newline = data.firstIndex(of: 10), newline < 20, let length = Int(String(decoding: data[..<newline], as: UTF8.self)), length >= 0, length <= 32 << 20 else { throw SourceFailure.corrupt }
        let start = newline + 1
        let available = min(length, data.count - start)
        guard let body = String(data: data.subdata(in: start..<(start + available)), encoding: .isoLatin1) else { throw SourceFailure.corrupt }
        var partial = file.lastPathComponent.contains(".partial.") || available < length
        let decoded = try mime(body, depth: 0, partial: &partial)
        if decoded.0.isEmpty && decoded.1.isEmpty { partial = true }
        var plain = decoded.0
        if plain.isEmpty && !decoded.1.isEmpty {
            let raw = decoded.1.replacingOccurrences(of: "<pre>", with: "").replacingOccurrences(of: "</pre>", with: "").replacingOccurrences(of: "&lt;", with: "<").replacingOccurrences(of: "&gt;", with: ">").replacingOccurrences(of: "&quot;", with: "\"").replacingOccurrences(of: "&#39;", with: "'").replacingOccurrences(of: "&amp;", with: "&")
            plain = raw.replacingOccurrences(of: "(?is)<(script|style)\\b[^>]*>.*?</\\1\\s*>", with: "", options: .regularExpression).replacingOccurrences(of: "<[^>]*>", with: "", options: .regularExpression)
        }
        return result(partial ? "partial" : "downloaded", plain, decoded.1)
    }
    private static func result(_ state: String, _ text: String, _ html: String) -> JSON {
        .object(["state": .string(state), "plain_text": .string(text), "html": .string(html)])
    }
    private static func mime(_ input: String, depth: Int, partial: inout Bool) throws -> (String, String) {
        try Task.checkCancellation()
        guard depth < 16 else { throw SourceFailure.corrupt }
        let normalized = input.replacingOccurrences(of: "\r\n", with: "\n")
        guard let split = normalized.range(of: "\n\n") else { partial = true; return ("", "") }
        var headers: [String: String] = [:]
        var key = ""
        for line in normalized[..<split.lowerBound].components(separatedBy: "\n") {
            if line.hasPrefix(" ") || line.hasPrefix("\t") { headers[key, default: ""] += " " + line.trimmingCharacters(in: .whitespaces); continue }
            guard let colon = line.firstIndex(of: ":") else { continue }
            key = line[..<colon].lowercased()
            headers[key] = String(line[line.index(after: colon)...]).trimmingCharacters(in: .whitespaces)
        }
        let type = headers["content-type", default: "text/plain"].lowercased()
        let payload = String(normalized[split.upperBound...])
        if headers["content-disposition", default: ""].lowercased().hasPrefix("attachment") { return ("", "") }
        if type.hasPrefix("multipart/") {
            guard let range = headers["content-type", default: ""].range(of: "boundary=", options: .caseInsensitive) else { partial = true; return ("", "") }
            let tail = String(headers["content-type"]![range.upperBound...])
            let boundary = tail.hasPrefix("\"") ? String(tail.dropFirst().prefix { $0 != "\"" }) : String(tail.prefix { $0 != ";" && !$0.isWhitespace })
            guard !boundary.isEmpty, boundary.count <= 200 else { throw SourceFailure.corrupt }
            if !payload.contains("--" + boundary + "--") { partial = true }
            var text = "", html = ""
            for part in payload.components(separatedBy: "--" + boundary).dropFirst() {
                if part.hasPrefix("--") { break }
                var contents = part
                if contents.hasPrefix("\n") { contents.removeFirst() }
                if contents.hasSuffix("\n") { contents.removeLast() }
                let decoded = try mime(contents, depth: depth + 1, partial: &partial)
                text += decoded.0; html += decoded.1
            }
            return (text, html)
        }
        guard type.hasPrefix("text/plain") || type.hasPrefix("text/html") else { return ("", "") }
        guard var bytes = payload.data(using: .isoLatin1) else { throw SourceFailure.corrupt }
        let encoding = headers["content-transfer-encoding", default: ""].lowercased()
        if encoding == "base64" {
            guard let decoded = Data(base64Encoded: payload.filter { !$0.isWhitespace }) else { partial = true; return ("", "") }
            bytes = decoded
        } else if encoding == "quoted-printable" {
            let raw = Array(bytes); var output: [UInt8] = []; var i = 0
            while i < raw.count {
                if raw[i] == 61 {
                    if i + 1 < raw.count && raw[i+1] == 10 { i += 2; continue }
                    if i + 2 < raw.count, let byte = UInt8(String(decoding: raw[(i+1)...(i+2)], as: UTF8.self), radix: 16) { output.append(byte); i += 3; continue }
                    partial = true
                }
                output.append(raw[i]); i += 1
            }
            bytes = Data(output)
        } else if !["", "7bit", "8bit", "binary"].contains(encoding) { partial = true; return ("", "") }
        let text: String
        if type.contains("iso-8859-1") { text = String(data: bytes, encoding: .isoLatin1) ?? "" }
        else if let decoded = String(data: bytes, encoding: .utf8) { text = decoded }
        else { partial = true; return ("", "") }
        if type.hasPrefix("text/plain") { return (text, "") }
        let escaped = text.replacingOccurrences(of: "&", with: "&amp;").replacingOccurrences(of: "<", with: "&lt;").replacingOccurrences(of: ">", with: "&gt;").replacingOccurrences(of: "\"", with: "&quot;").replacingOccurrences(of: "'", with: "&#39;")
        return ("", "<pre>" + escaped + "</pre>")
    }
}
