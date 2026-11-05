import Foundation
import PDFKit
import AppKit
import SourceSupport

public struct PDFSource: Source {
    public let name = "pdf"
    public init() {}
    public func status() async -> JSON? { .object([:]) }
    public func primitive(_ operation: String, args: JSON) async throws -> JSON {
        switch operation {
        case "pdf.inspect":
            guard let document = PDFDocument(url: URL(fileURLWithPath: args["path"].string)) else { throw SourceFailure.invalid }
            var pages: [JSON] = []
            for index in 0..<document.pageCount {
                guard let bounds = document.page(at: index)?.bounds(for: .mediaBox) else { throw SourceFailure.corrupt }
                pages.append(.object(["x": .number(bounds.minX), "y": .number(bounds.minY), "width": .number(bounds.width), "height": .number(bounds.height)]))
            }
            return .object(["locked": .bool(document.isLocked), "page_count": .number(Double(document.pageCount)), "pages": .array(pages)])
        case "pdf.annotate":
            return try annotate(args)
        default:
            throw SourceFailure.unsupported
        }
    }
    private func annotate(_ args: JSON) throws -> JSON {
        guard let document = PDFDocument(url: URL(fileURLWithPath: args["input"].string)) else { throw SourceFailure.invalid }
        let items = args["annotations"].array
        for item in items {
            guard let page = document.page(at: Int(item["page"].number) - 1) else { throw SourceFailure.invalid }
            let rect = CGRect(x: item["x"].number, y: item["y"].number, width: item["width"].number, height: item["height"].number)
            let annotation = PDFAnnotation(bounds: rect, forType: item["type"].string == "note" ? .text : .highlight, withProperties: nil)
            annotation.contents = item["text"].string
            if item["type"].string == "highlight" { annotation.quadrilateralPoints = [NSValue(point: CGPoint(x: 0, y: rect.height)), NSValue(point: CGPoint(x: rect.width, y: rect.height)), NSValue(point: .zero), NSValue(point: CGPoint(x: rect.width, y: 0))] }
            page.addAnnotation(annotation)
        }
        guard let bytes = document.dataRepresentation(), let output = PDFDocument(data: bytes) else { throw SourceFailure.corrupt }
        try bytes.write(to: URL(fileURLWithPath: args["output"].string), options: .withoutOverwriting)
        let found: [JSON] = items.map { item in
            let kind = item["type"].string == "note" ? "Text" : "Highlight"
            return .bool(output.page(at: Int(item["page"].number) - 1)?.annotations.contains { ($0.contents ?? "") == item["text"].string && $0.type == kind } == true)
        }
        return .object(["found": .array(found)])
    }
}
