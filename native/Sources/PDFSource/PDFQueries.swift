import Foundation
import PDFKit
import SourceSupport

public enum PDFQueries {
    public static func run(_ operation: String, args: JSON) async throws -> JSON {
        let data = try queryDocument(args)
        guard Set(args.object.keys).isSubset(of: ["data_base64", "file", "text"]),
              let document = PDFDocument(data: data), document.pageCount <= 1000, !document.isLocked else { throw SourceFailure.invalid }
        var pages: [JSON] = []
        var budget = 200000
        if operation == "pdf.search" && (args["text"].string.isEmpty || args["text"].string.count > 1000) { throw SourceFailure.invalid }
        for index in 0..<document.pageCount {
            try Task.checkCancellation()
            guard let page = document.page(at: index) else { throw SourceFailure.corrupt }
            let text = page.string ?? ""
            budget -= text.utf8.count
            guard budget >= 0 else { throw SourceFailure.invalid }
            if operation == "pdf.text" {
                pages.append(.object(["page": .integer(UInt64(index + 1)), "text": .string(text)]))
            } else {
                var range = text.startIndex..<text.endIndex
                while let match = text.range(of: args["text"].string, options: .caseInsensitive, range: range) {
                    guard pages.count < 10000 else { throw SourceFailure.invalid }
                    pages.append(.object(["page": .integer(UInt64(index + 1)), "offset": .integer(UInt64(text.utf16.distance(from: text.utf16.startIndex, to: match.lowerBound))), "length": .integer(UInt64(text[match].utf16.count))]))
                    range = match.upperBound..<text.endIndex
                }
            }
        }
        return .object([operation == "pdf.text" ? "pages" : "matches": .array(pages), "page_count": .integer(UInt64(document.pageCount))])
    }
}
