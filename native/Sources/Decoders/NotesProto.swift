import Foundation

public enum NotesProto {
    public static func decode(_ data: Data, tables: [String: Data] = [:]) throws -> NotesBody {
        let root = try NotesProtoMessage(NotesProtoWire.unpack(data))
        try root.validate([2: 2])
        guard let document = try root.message(2), let note = try document.message(3) else { throw NotesProtoFailure.malformed }
        try document.validate([2: 0, 3: 2])
        return try decodeNote(note, tables: tables, depth: 0)
    }

    static func decodeNote(_ note: NotesProtoMessage, tables: [String: Data], depth: Int) throws -> NotesBody {
        try note.validate([2: 2, 3: 2, 4: 2, 5: 2], repeated: [3, 5])
        guard depth < 8, let text = try note.string(2) else { throw NotesProtoFailure.malformed }
        let attributes = try note.repeated(5).map(NotesProtoMessage.init)
        var attachments: [String] = []
        for attribute in attributes {
            try attribute.validate([1: 0, 2: 2, 3: 2, 5: 0, 6: 0, 7: 0, 8: 0, 9: 2, 10: 2, 12: 2, 13: 0, 14: 0])
            if let identifier = try attribute.message(12)?.string(1), !attachments.contains(identifier) { attachments.append(identifier) }
        }
        if attributes.contains(where: { $0.integer(1) == nil }) {
            let block = NotesBlock(type: "unknown", text: text, runs: [NotesRun(text: text)], unsupported: "missing_run_length")
            return NotesBody(text: text, rich: true, attachments: attachments, blocks: [block], html: escape(text), lossless: false)
        }
        let units = Array(text.utf16)
        var offset = 0
        var complete = note.fields.allSatisfy { [2, 3, 4, 5].contains($0.number) }
        var flat: [NotesBlock] = []
        var current = NotesBlock(type: "paragraph", text: "", runs: [])
        func append(_ run: NotesRun, paragraph: NotesBlock?) {
            let pieces = run.text.components(separatedBy: "\n")
            for (index, piece) in pieces.enumerated() {
                if index > 0 && index == pieces.count - 1 && piece.isEmpty { break }
                if let paragraph {
                    let previousText = current.text, previousRuns = current.runs
                    let previousUnsupported = current.unsupported
                    current = paragraph
                    current.text = previousText
                    current.runs = previousRuns
                    if current.unsupported == nil { current.unsupported = previousUnsupported }
                }
                if !piece.isEmpty {
                    var part = run
                    part.text = piece
                    current.runs.append(part)
                    current.text += piece
                }
                if index < pieces.count - 1 {
                    flat.append(current)
                    current = NotesBlock(type: "paragraph", text: "", runs: [])
                }
            }
        }
        for attribute in attributes {
            guard let count = attribute.integer(1), count <= UInt64(units.count - offset) else { throw NotesProtoFailure.malformed }
            let end = offset + Int(count)
            for boundary in [offset, end] where boundary > 0 && boundary < units.count {
                guard !(0xdc00...0xdfff).contains(units[boundary]) || !(0xd800...0xdbff).contains(units[boundary - 1]) else { throw NotesProtoFailure.malformed }
            }
            let segment = String(decoding: units[offset..<end], as: UTF16.self)
            offset = end
            var run = NotesRun(text: segment)
            var paragraph: NotesBlock?
            do {
                run = try inline(attribute, text: segment)
                paragraph = try attribute.message(2).map(paragraphBlock)
            } catch NotesProtoFailure.unsupported {
                run = NotesRun(text: segment)
                paragraph = NotesBlock(type: "unknown", text: "", runs: [], unsupported: "unsupported_attributes")
            }
            if attribute.fields.contains(where: { ![1, 2, 3, 5, 6, 7, 8, 9, 10, 12, 13, 14].contains($0.number) }) {
                complete = false
                if paragraph == nil { paragraph = NotesBlock(type: "paragraph", text: "", runs: []) }
                paragraph?.unsupported = "unknown_attribute"
            }
            if let unsupported = run.unsupported {
                if paragraph == nil { paragraph = NotesBlock(type: "paragraph", text: "", runs: []) }
                paragraph?.unsupported = unsupported
            }
            if paragraph?.unsupported != nil { complete = false }
            append(run, paragraph: paragraph)
        }
        if offset < units.count {
            append(NotesRun(text: String(decoding: units[offset...], as: UTF16.self)), paragraph: nil)
            complete = complete && attributes.isEmpty
        }
        if !current.runs.isEmpty || flat.isEmpty { flat.append(current) }
        if note.fields.contains(where: { ![2, 3, 4, 5].contains($0.number) }) {
            flat.append(NotesBlock(type: "unknown", text: "", runs: [], unsupported: "unknown_note_field"))
        }
        for index in flat.indices {
            let ids = flat[index].runs.compactMap(\.attachment_id)
            if flat[index].text == "\u{fffc}", ids.count == 1 {
                flat[index].attachment_id = ids[0]
                let uti = flat[index].runs.first?.attachment_uti ?? ""
                if uti == "com.apple.notes.table" {
                    flat[index].type = "table"
                    if let data = tables[ids[0]], let rows = try? table(data, tables: tables, depth: depth + 1) {
                        flat[index].rows = rows
                    } else { flat[index].unsupported = "table_data_unavailable" }
                } else if flat[index].type == "paragraph" { flat[index].type = "attachment" }
                complete = false
            } else if flat[index].text.contains("\u{fffc}"), ids.isEmpty {
                flat[index].type = "unknown"
                flat[index].unsupported = "attachment_identifier_missing"
                complete = false
            }
        }
        var position = 0
        func sections(_ level: Int) -> [NotesBlock] {
            var blocks: [NotesBlock] = []
            while position < flat.count {
                var block = flat[position]
                let heading = block.type == "heading" ? block.level ?? 1 : nil
                if let heading, heading <= level { break }
                position += 1
                if let heading {
                    block.collapsible = true
                    block.children = sections(heading)
                }
                blocks.append(block)
            }
            return blocks
        }
        let blocks = sections(0)
        func inventory(_ blocks: [NotesBlock]) {
            for block in blocks {
                for run in block.runs {
                    if let identifier = run.attachment_id, !attachments.contains(identifier) { attachments.append(identifier) }
                }
                inventory(block.children)
                for row in block.rows ?? [] { for cell in row { inventory(cell.blocks) } }
            }
        }
        inventory(blocks)
        let rich = flat.enumerated().contains { index, block in
            if block.unsupported != nil || !block.runs.allSatisfy(plainRun) { return true }
            if block.blockquote == true || (block.alignment != nil && block.alignment != "left") || (block.type != "heading" && (block.level ?? 0) > 0) { return true }
            return block.type != "paragraph" && !(index == 0 && block.type == "title")
        } || !attachments.isEmpty
        let roundTrip = flat.allSatisfy { block in
            guard ["paragraph", "title", "heading", "monostyled"].contains(block.type), block.blockquote != true, block.level == nil || block.type == "heading" else { return false }
            return block.runs.allSatisfy { run in
                guard run.highlight == nil, run.font == nil, run.font_size == nil, run.color == nil, run.superscript == nil, !run.text.contains("\u{0}"), !run.text.contains("\u{2028}") else { return false }
                guard let link = run.link else { return true }
                return ["http", "https", "mailto", "notes"].contains(URL(string: link)?.scheme?.lowercased() ?? "")
            }
        }
        return NotesBody(text: text, rich: rich, attachments: attachments, blocks: blocks, html: html(blocks), lossless: complete && attachments.isEmpty && roundTrip)
    }

    private static func plainRun(_ run: NotesRun) -> Bool {
        run.unsupported == nil && !run.bold && !run.italic && !run.underline && !run.strikethrough && run.font == nil && run.font_size == nil && run.link == nil && run.highlight == nil && run.color == nil && run.superscript == nil && run.attachment_id == nil
    }
}
