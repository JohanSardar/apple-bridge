import Foundation
import CryptoKit
import Darwin
import SourceSupport
import AgendaGate

@AgendaActor
public final class CalendarStoreEnrichment {
    static let fields: Set<String> = ["store_status", "store_coverage", "store_identifier_match", "travel_time", "travel_advisory_behavior", "structured_location", "invitation_status", "owner_participation_status", "privacy_level", "priority", "conference_url", "attachments", "start_location", "attendee_proposals", "participant_comments", "store_alarm_details"]
    private let root: URL
    public nonisolated init(root: URL = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Group Containers/group.com.apple.calendar", isDirectory: true)) { self.root = root }

    private var snapshot: CalendarStoreSnapshot?
    private var signature: [String] = []
    private var blobs: [String: CalendarAttachmentBlob] = [:]
    private var files: [String: CalendarAttachmentBlob] = [:]
    public private(set) var handles: [String: String] = [:]

    private func refresh(force: Bool) async -> Bool {
        let path = root.appendingPathComponent("Calendar.sqlitedb").path
        let current = [path, path + "-wal"].map { path in
            var metadata = stat()
            guard Darwin.lstat(path, &metadata) == 0 else { return "missing" }
            return "\(metadata.st_ino):\(metadata.st_size):\(metadata.st_mtimespec.tv_sec):\(metadata.st_mtimespec.tv_nsec)"
        }
        if !force, snapshot != nil, current == signature { return true }
        do {
            snapshot = try await Task.detached { try CalendarStoreSnapshot.read(path: path) }.value
            signature = current
            return true
        } catch { snapshot = nil; return false }
    }

    public func enrich(_ observations: [String: JSON], lookups: [String: JSON], force: Bool) async -> [String: JSON] {
        let ready = await refresh(force: force)
        var result = observations
        for (identity, observation) in observations where observation["collection"].string == "calendar.event" {
            await Task.yield()
            var value = observation.object
            var data = observation["data"].object
            let native = observation["native_key"].string
            let identifier = String(native.split(separator: "|", maxSplits: 1).first ?? "")
            let calendar = observation["data"]["calendar_id"]["$ref"]["native_key"].string
            let row = ready ? snapshot?.event(identifier: identifier, external: lookups[native]?["external"].string ?? "", calendar: calendar, detached: observation["data"]["is_exception"].bool) : nil
            var coverage: [String: JSON] = [:]
            func set(_ key: String, _ value: JSON, status: String = "complete", reason: String = "") {
                data[key] = value
                coverage[key] = .object(["status": .string(status), "reason": .string(reason)])
            }
            if let row {
                set("store_identifier_match", row["matched_by"] ?? .null)
                for field in ["start_location", "attendee_proposals", "participant_comments", "store_alarm_details"] { set(field, .null, status: "unsupported", reason: "positive_native_fixture_not_verified_on_this_os") }
                for field in ["travel_advisory_behavior", "invitation_status", "privacy_level", "priority"] { set(field, row[field] ?? .null) }
                set("owner_participation_status", row["self_status"] ?? .null)
                let travel = row["travel_time"] ?? .null
                let knownZero: Bool
                switch travel {
                case .number(let value): knownZero = value == 0
                case .integer(let value): knownZero = value == 0
                default: knownZero = false
                }
                set("travel_time", knownZero ? travel : .null, status: knownZero ? "complete" : "unsupported", reason: knownZero ? "" : travel == .null ? "travel_time_unavailable" : "positive_travel_time_units_not_verified_on_this_os")
                if row["location_title"] == .null && row["location_address"] == .null { set("structured_location", .null) }
                else {
                    set("structured_location", .object(["title": row["location_title"] ?? .null, "address": row["location_address"] ?? .null, "latitude": .null, "longitude": .null, "radius": .null]), status: "partial", reason: "coordinate_units_not_verified_on_this_os")
                }
                let conference = row["conference_url"] ?? .null
                if conference == .null || conference.string.isEmpty { set("conference_url", .null) }
                else if let url = URL(string: conference.string), ["https", "http"].contains(url.scheme?.lowercased() ?? "") { set("conference_url", conference) }
                else { set("conference_url", .null, status: "unsupported", reason: "unsupported_conference_url_scheme") }
                let rows = snapshot?.attachments[Int(exactly: row["item_id"]?.number ?? 0) ?? 0] ?? []
                var attachments: [JSON] = []
                var complete = true
                for item in rows {
                    do {
                        let blob = try await attachment(item)
                        attachments.append(.object(["filename": item["filename"] ?? .string("Attachment"), "mime_type": .string(blob.mime), "blob": .object(["state": .string("present"), "digest": .string(blob.digest), "size": .number(Double(blob.size))]), "derivatives": .object([:])]))
                        handles[blob.digest] = blob.handle
                    } catch { complete = false }
                }
                if !rows.isEmpty { coverage["attachment_derivatives"] = .object(["status": .string("unsupported"), "reason": .string("native_calendar_derivatives_not_verified")]) }
                set("attachments", .array(attachments), status: complete ? "complete" : "partial", reason: complete ? "" : "attachment_bytes_not_locally_available_or_path_unverified")
                data["store_status"] = .string(coverage.values.allSatisfy { $0["status"].string == "complete" } ? "complete" : "partial")
            } else {
                for field in Self.fields.subtracting(["store_status", "store_coverage"]) { set(field, .null, status: "unsupported", reason: ready ? "calendar_store_identity_unverified" : "calendar_store_unavailable_or_schema_unverified") }
                data["store_status"] = .string("unsupported")
            }
            data["store_coverage"] = .object(coverage)
            value["data"] = .object(data)
            result[identity] = .object(value)
        }
        return result
    }

    private func attachment(_ row: [String: JSON]) async throws -> CalendarAttachmentBlob {
        let path = row["local_path"]?.string ?? ""
        guard !path.isEmpty else { throw SourceFailure.unavailable }
        let url = (path.hasPrefix("file:") ? URL(string: path) : (path.hasPrefix("/") ? URL(fileURLWithPath: path) : root.appendingPathComponent(path)))?.resolvingSymlinksInPath()
        guard let url, url.isFileURL, url.path.hasPrefix(root.resolvingSymlinksInPath().path + "/") else { throw SourceFailure.unsupported }
        let attributes = try FileManager.default.attributesOfItem(atPath: url.path)
        let size = (attributes[.size] as? NSNumber)?.int64Value ?? -1
        let modified = attributes[.modificationDate] as? Date ?? .distantPast
        guard attributes[.type] as? FileAttributeType == .typeRegular, size >= 0 else { throw SourceFailure.unavailable }
        var metadata = stat()
        guard Darwin.lstat(url.path, &metadata) == 0, metadata.st_flags & UInt32(SF_DATALESS) == 0 else { throw SourceFailure.unavailable }
        if let cached = files[url.path], cached.size == size, cached.modified == modified { return cached }
        let digest = try await Task.detached {
            let file = try Self.openLocalFile(url)
            defer { try? file.close() }
            var hash = SHA256()
            while let bytes = try file.read(upToCount: 128 * 1024), !bytes.isEmpty { try Task.checkCancellation(); hash.update(data: bytes); await Task.yield() }
            return hash.finalize().map { String(format: "%02x", $0) }.joined()
        }.value
        let after = try FileManager.default.attributesOfItem(atPath: url.path)
        guard (after[.size] as? NSNumber)?.int64Value == size, after[.modificationDate] as? Date == modified else { throw SourceFailure.unavailable }
        let mime: String
        switch url.pathExtension.lowercased() {
        case "pdf": mime = "application/pdf"
        case "png": mime = "image/png"
        case "jpg", "jpeg": mime = "image/jpeg"
        case "txt": mime = "text/plain"
        default: mime = "application/octet-stream"
        }
        let blob = CalendarAttachmentBlob(url: url, size: size, modified: modified, digest: digest, handle: "calendar-attachment:" + UUID().uuidString, mime: mime)
        files[url.path] = blob; blobs[blob.handle] = blob
        return blob
    }

    private nonisolated static func openLocalFile(_ url: URL) throws -> FileHandle {
        let descriptor = Darwin.open(url.path, O_RDONLY | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK)
        guard descriptor >= 0 else { throw SourceFailure.unavailable }
        var metadata = stat()
        guard Darwin.fstat(descriptor, &metadata) == 0, metadata.st_mode & S_IFMT == S_IFREG, metadata.st_flags & UInt32(SF_DATALESS) == 0 else {
            Darwin.close(descriptor)
            throw SourceFailure.unavailable
        }
        return FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
    }

    public func readBlob(_ handle: String, offset: Int, length: Int) throws -> JSON {
        guard let blob = blobs[handle], offset >= 0, Int64(offset) <= blob.size, length > 0, length <= 131072 else { throw SourceFailure.unavailable }
        var metadata = stat()
        guard Darwin.lstat(blob.url.path, &metadata) == 0, metadata.st_flags & UInt32(SF_DATALESS) == 0,
              blob.url.resolvingSymlinksInPath().path.hasPrefix(root.resolvingSymlinksInPath().path + "/") else { throw SourceFailure.unavailable }
        let attributes = try FileManager.default.attributesOfItem(atPath: blob.url.path)
        guard (attributes[.size] as? NSNumber)?.int64Value == blob.size, attributes[.modificationDate] as? Date == blob.modified else { throw SourceFailure.unavailable }
        let file = try Self.openLocalFile(blob.url)
        defer { try? file.close() }
        try file.seek(toOffset: UInt64(offset))
        let bytes = try file.read(upToCount: length) ?? Data()
        return .object(["t": .string("BlobChunk"), "handle": .string(handle), "offset": .number(Double(offset)), "bytes": .string(bytes.base64EncodedString()), "digest": .string(blob.digest), "eof": .bool(Int64(offset + bytes.count) == blob.size)])
    }
}

private struct CalendarAttachmentBlob: Sendable {
    let url: URL
    let size: Int64
    let modified: Date
    let digest: String
    let handle: String
    let mime: String
}
