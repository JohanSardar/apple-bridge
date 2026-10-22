import Foundation
import Decoders

@main
struct NotesDecoderChecks {
    @MainActor
    static func main() async throws {
        let root = CommandLine.arguments[1]
        for (name, rich) in [("plain", false), ("formatting", true), ("checklist", true), ("table", true), ("attachment", true)] {
            let bytes = try Data(contentsOf: URL(fileURLWithPath: root + "/" + name + ".gz"))
            let decoded = try NotesProto.decode(bytes)
            guard decoded.text == "[ab-test decoder] Title\nbody 🧪\n", decoded.rich == rich else { throw CheckFailure.mismatch }
            for length in 0..<bytes.count {
                guard (try? NotesProto.decode(bytes.prefix(length))) == nil else { throw CheckFailure.truncation }
            }
        }
        guard (try? NotesProto.decode(Data(contentsOf: URL(fileURLWithPath: root + "/malformed.gz")))) == nil else { throw CheckFailure.malformed }
        try full(root)
        print("Notes decoder fixtures passed")
    }
    private struct Expected: Decodable {
        let text: String
        let types: [String]
        let table: [[String]]
        let attachments: [String]
    }
    private static func full(_ root: String) throws {
        let directory = URL(fileURLWithPath: root)
        let expected = try JSONDecoder().decode(Expected.self, from: Data(contentsOf: directory.appendingPathComponent("full.json")))
        let table = try Data(contentsOf: directory.appendingPathComponent("full-table.gz"))
        let bytes = try Data(contentsOf: directory.appendingPathComponent("full.gz"))
        let body = try NotesProto.decode(bytes, tables: ["fixture-table": table])
        func flatten(_ blocks: [NotesBlock]) -> [NotesBlock] { blocks.flatMap { [$0] + flatten($0.children) } }
        let blocks = flatten(body.blocks)
        guard body.text == expected.text, body.rich, !body.lossless,
              blocks.map(\.type) == expected.types, body.attachments == expected.attachments,
              blocks[1].collapsible == true, blocks[1].children.first?.type == "heading",
              blocks[2].level == 2, blocks[4].list_style == "bullet", blocks[4].level == 1,
              blocks[5].list_style == "dash", blocks[6].list_style == "numbered",
              blocks[7].checked == false, blocks[8].checked == true,
              blocks[7].checklist_id != blocks[8].checklist_id,
              blocks[9].blockquote == true else { throw CheckFailure.mismatch }
        let styled = blocks[10].runs[0]
        guard styled.bold, styled.italic, styled.underline, styled.strikethrough,
              styled.highlight == "pink", styled.link == "https://example.org/?a=1&b=2",
              body.html.contains("Code &lt;&amp;&gt;"), body.html.contains("a=1&amp;b=2") else { throw CheckFailure.mismatch }
        let cells = blocks[12].rows?.map { $0.map { $0.blocks.map(\.text).joined() } }
        guard cells == expected.table, blocks[12].unsupported == nil else { throw CheckFailure.mismatch }
        let missing = flatten(try NotesProto.decode(bytes).blocks)
        guard missing[12].type == "table", missing[12].unsupported == "table_data_unavailable",
              flatten(try NotesProto.decode(bytes, tables: ["fixture-table": Data([0xff])]).blocks)[12].rows == nil else { throw CheckFailure.mismatch }
        for length in 0..<table.count {
            let truncated = flatten(try NotesProto.decode(bytes, tables: ["fixture-table": table.prefix(length)]).blocks)
            guard truncated[12].rows == nil else { throw CheckFailure.truncation }
        }
        let roundTrip = try JSONDecoder().decode([NotesBlock].self, from: JSONEncoder().encode(body.blocks))
        guard roundTrip == body.blocks else { throw CheckFailure.mismatch }
        for name in ["surrogate-split", "duplicate-text", "wrong-weight-wire"] {
            guard (try? NotesProto.decode(Data(contentsOf: directory.appendingPathComponent(name + ".gz")))) == nil else { throw CheckFailure.malformed }
        }
        let future = try NotesProto.decode(Data(contentsOf: directory.appendingPathComponent("unknown-style.gz")))
        guard future.text == "[ab-test] Future style", !future.lossless, future.rich,
              future.blocks.first?.unsupported == "unknown_paragraph_style" else { throw CheckFailure.mismatch }
        print("Full Notes blocks: \(blocks.count), checklists: 2, table cells: 4, attachments: \(body.attachments.count)")
    }
    private enum CheckFailure: Error { case mismatch, truncation, malformed }
}
