import Foundation
import SourceSupport

public actor MailSource: Source {
    public nonisolated let name = "mail"
    private let path: String
    private var generation = ""
    private var keys = Set<String>()
    public static let coverage: JSON = .object(["fields": .string("partial"), "missing": .array([.string("account settings, recipients, attachment files and local body availability require qualification")])])
    public init(path: String = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Mail/V10/MailData/Envelope Index").path) { self.path = path }
    public func open() async -> SourceReadiness {
        do {
            let reader = try SQLiteSnapshot(path: path, generationSQL: "SELECT 'mail-v10'")
            try reader.requireSchema(["messages": ["mailbox", "subject", "sender", "date_received", "read", "message_id"], "mailboxes": ["url"], "subjects": ["subject"], "addresses": ["address"]])
            return SourceReadiness(state: "ready", coverage: Self.coverage)
        } catch { return SourceReadiness(state: "permission_missing", coverage: Self.coverage, reason: "Full Disk Access and a supported V10 Envelope Index required") }
    }
    public func snapshot(maxRows: Int = 100000) throws -> (String, [JSON], Bool) {
        let reader = try SQLiteSnapshot(path: path, generationSQL: "SELECT 'mail-v10'")
        let scan = try CompleteScan.read(reader: reader, sql: "SELECT CAST(m.ROWID AS TEXT) AS native_key, m.mailbox, b.url, coalesce(s.subject,'') AS subject, coalesce(a.address,'') AS sender, m.date_received, m.read, coalesce(m.message_id,'') AS message_id FROM messages m LEFT JOIN mailboxes b ON b.ROWID=m.mailbox LEFT JOIN subjects s ON s.ROWID=m.subject LEFT JOIN addresses a ON a.ROWID=m.sender ORDER BY m.ROWID", keyColumn: "native_key", scope: "mail.message", previousScope: "mail.message", previousGeneration: generation, previousKeys: keys, maxRows: maxRows, timeout: 5)
        var observations: [JSON] = []
        for row in scan.rows {
            guard row["url"]?.text != nil else { throw SourceFailure.corrupt }
            let key = row["native_key"]?.text ?? ""
            let alias = row["message_id"]?.text ?? ""
            let data: JSON = .object(["subject": .string(row["subject"]?.text ?? ""), "from": .string(row["sender"]?.text ?? ""), "received_at": .string(timestamp(Date(timeIntervalSince1970: Self.numeric(row["date_received"])))), "read": .bool(Self.numeric(row["read"]) != 0), "message_id": .string(alias), "mailbox_ids": .array([.string(String(Int(Self.numeric(row["mailbox"]))))]), "account_id": .string(Self.account(row["url"]?.text ?? "")), "to": .null, "cc": .null, "sent_at": .null, "flagged": .null, "flag_color": .null, "has_attachments": .null, "size": .null, "snippet": .null, "body_state": .null])
            var observation: [String: JSON] = ["kind": .string("upsert"), "collection": .string("mail.message"), "native_key": .string(key), "data": data, "observed_at": .string(timestamp())]
            if !alias.isEmpty { observation["scoped_alias"] = .string("mail/" + data["account_id"].string + "/" + alias) }
            observations.append(.object(observation))
        }
        if scan.complete {
            for key in scan.absentKeys { observations.append(.object(["kind": .string("absent"), "revalidated": .bool(true), "collection": .string("mail.message"), "native_key": .string(key), "observed_at": .string(timestamp())])) }
            keys = Set(scan.rows.compactMap { $0["native_key"]?.text })
            generation = reader.generation
        }
        return (reader.generation, observations, scan.complete)
    }
    private static func account(_ value: String) -> String {
        guard let url = URL(string: value), let host = url.host else { return "unknown" }
        return (url.user.map { $0 + "@" } ?? "") + host
    }
    private static func numeric(_ value: SQLiteValue?) -> Double {
        switch value { case .integer(let n): return Double(n); case .real(let n): return n; default: return 0 }
    }
    public func capture(_ resume: JSON, into sink: any ObservationSink) async throws {
        let watcher = DatabaseWatcher(path: path)
        defer { watcher.close() }
        var previous: [JSON] = []
        func publish() async throws {
            let started = timestamp()
            let (generation, observations, complete) = try snapshot()
            let unstamped = observations.map { value -> JSON in var object = value.object; object.removeValue(forKey: "observed_at"); return .object(object) }
            if unstamped != previous {
                try await sink.emitCheckpoint(source: name, generation: generation, checkpoint: UUID().uuidString, observations: observations, scanID: UUID().uuidString, scanStartedAt: started, scope: ["mail.message"], completionEvidence: complete ? "complete" : "partial", blobHandles: [:], live: !previous.isEmpty)
                previous = unstamped
            }
        }
        try await publish()
        for await _ in watcher.changes { try Task.checkCancellation(); try await publish() }
    }
    public func prepare(_ command: JSON) async throws -> Prepared { throw SourceFailure.unsupported }
    public func dispatch(_ prepared: Prepared) async throws -> JSON { throw SourceFailure.unsupported }
    public func resolve(_ command: String, evidence: JSON) async -> JSON { .null }
}
