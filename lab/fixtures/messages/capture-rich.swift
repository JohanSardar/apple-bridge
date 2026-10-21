import Foundation
import SQLite3
import Decoders

let directory = URL(fileURLWithPath: CommandLine.arguments[1])
var database: OpaquePointer?
guard sqlite3_open_v2(NSHomeDirectory() + "/Library/Messages/chat.db", &database, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else { exit(1) }
defer { sqlite3_close(database) }
for (label, needle) in [("formatting", "__kIMTextBoldAttributeName"), ("effect", "__kIMTextEffectAttributeName"), ("genmoji", "__kIMEmojiImageAttributeName")] {
    var statement: OpaquePointer?
    guard sqlite3_prepare_v2(database, "SELECT attributedBody FROM message WHERE instr(attributedBody,cast(? as blob))>0 LIMIT 10", -1, &statement, nil) == SQLITE_OK else { exit(1) }
    _ = needle.withCString { sqlite3_bind_text(statement, 1, $0, -1, unsafeBitCast(-1, to: sqlite3_destructor_type.self)) }
    var saved = false
    while sqlite3_step(statement) == SQLITE_ROW && !saved {
        guard let pointer = sqlite3_column_blob(statement, 0) else { continue }
        let data = Data(bytes: pointer, count: Int(sqlite3_column_bytes(statement, 0)))
        guard let scrubbed = try? FixtureScrubber.scrub(data), TypedStream.decode(scrubbed).decodeStatus == "decoded" else { continue }
        try scrubbed.write(to: directory.appendingPathComponent("rich-air-" + label + ".typedstream"))
        saved = true
    }
    sqlite3_finalize(statement)
    guard saved else { exit(1) }
}
print("scrubbed rich typedstream fixtures captured")
