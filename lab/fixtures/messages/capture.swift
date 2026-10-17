import Foundation
import SQLite3
import Decoders

let directory = URL(fileURLWithPath: CommandLine.arguments[1])
let path = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/chat.db").path
var connection: OpaquePointer?
guard sqlite3_open_v2(path, &connection, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else { exit(1) }
defer { sqlite3_close(connection) }
var statement: OpaquePointer?
guard sqlite3_prepare_v2(connection, "SELECT attributedBody FROM message WHERE attributedBody IS NOT NULL ORDER BY length(attributedBody) DESC LIMIT 1000", -1, &statement, nil) == SQLITE_OK else { exit(1) }
defer { sqlite3_finalize(statement) }
var saved = 0
var digests: Set<String> = []
while sqlite3_step(statement) == SQLITE_ROW && saved < 12 {
    guard let pointer = sqlite3_column_blob(statement, 0) else { continue }
    let data = Data(bytes: pointer, count: Int(sqlite3_column_bytes(statement, 0)))
    guard let scrubbed = try? FixtureScrubber.scrub(data), TypedStream.decode(scrubbed).decodeStatus == "decoded" else { continue }
    let digest = BodyDigest.hash(scrubbed)
    if digests.insert(digest).inserted {
        try scrubbed.write(to: directory.appendingPathComponent("air-\(saved).typedstream"))
        saved += 1
    }
}
print("scrubbed_fixtures", saved)

var edits: OpaquePointer?
guard sqlite3_prepare_v2(connection, "SELECT message_summary_info FROM message WHERE message_summary_info IS NOT NULL AND date_edited>0 LIMIT 12", -1, &edits, nil) == SQLITE_OK else { exit(1) }
defer { sqlite3_finalize(edits) }
var summaries = 0
while sqlite3_step(edits) == SQLITE_ROW && summaries < 3 {
    guard let pointer = sqlite3_column_blob(edits, 0) else { continue }
    let data = Data(bytes: pointer, count: Int(sqlite3_column_bytes(edits, 0)))
    guard let scrubbed = try? FixtureScrubber.summary(data), MessageSummary.decode(scrubbed).decodeStatus == "decoded" else { continue }
    try scrubbed.write(to: directory.appendingPathComponent("air-edited-\(summaries).plist"))
    summaries += 1
}
print("scrubbed_summaries", summaries)

for (label, needle) in [("mention", "__kIMMentionConfirmedMention"), ("attachment", "__kIMFileTransferGUIDAttributeName")] {
    var sample: OpaquePointer?
    guard sqlite3_prepare_v2(connection, "SELECT attributedBody FROM message WHERE instr(attributedBody,cast(? as blob))>0 LIMIT 20", -1, &sample, nil) == SQLITE_OK else { exit(1) }
    let transient = unsafeBitCast(-1, to: sqlite3_destructor_type.self)
    _ = needle.withCString { sqlite3_bind_text(sample, 1, $0, -1, transient) }
    var count = 0
    while sqlite3_step(sample) == SQLITE_ROW && count < 3 {
        guard let pointer = sqlite3_column_blob(sample, 0) else { continue }
        let data = Data(bytes: pointer, count: Int(sqlite3_column_bytes(sample, 0)))
        guard let scrubbed = try? FixtureScrubber.scrub(data), TypedStream.decode(scrubbed).decodeStatus == "decoded" else { continue }
        try scrubbed.write(to: directory.appendingPathComponent("air-\(label)-\(count).typedstream"))
        count += 1
    }
    sqlite3_finalize(sample)
    print("scrubbed_" + label, count)
}
