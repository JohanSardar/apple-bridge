import Foundation

public struct NotesRun: Codable, Sendable, Equatable {
    public var text: String
    public var bold = false
    public var italic = false
    public var underline = false
    public var strikethrough = false
    public var font: String?
    public var font_size: Double?
    public var color: String?
    public var superscript: Int?
    public var link: String?
    public var highlight: String?
    public var attachment_id: String?
    public var attachment_uti: String?
    public var unsupported: String?
}

public struct NotesCell: Codable, Sendable, Equatable {
    public var blocks: [NotesBlock]
}

public struct NotesBlock: Codable, Sendable, Equatable {
    public var type: String
    public var text: String
    public var runs: [NotesRun]
    public var level: Int?
    public var alignment: String?
    public var blockquote: Bool?
    public var list_style: String?
    public var checked: Bool?
    public var checklist_id: String?
    public var attachment_id: String?
    public var rows: [[NotesCell]]?
    public var children: [NotesBlock] = []
    public var collapsible: Bool?
    public var unsupported: String?
}

public struct NotesBody: Sendable {
    public let text: String
    public let rich: Bool
    public let attachments: [String]
    public let blocks: [NotesBlock]
    public let html: String
    public let lossless: Bool
}
