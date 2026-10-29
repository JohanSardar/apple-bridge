import Foundation
import PDFKit
import AppKit
import CryptoKit
import SourceSupport

public actor PDFSource: Source {
    public nonisolated let name = "pdf"
    private let writer: BlobWriter
    private var artifacts: [String: (JSON, StagedBlob)] = [:]
    private let generation = UUID().uuidString
    public init(home: URL) throws {
        writer = try BlobWriter(directory: home.appendingPathComponent("Library/Caches/AppleBridge/pdf-blobs"))
    }
    public func open() async -> SourceReadiness {
        SourceReadiness(state: "ready", coverage: .object(["fields": .string("complete")]), capabilities: ["pdf.annotate"])
    }
    public func capture(_ resume: JSON, into sink: any ObservationSink) async throws {
        try await sink.emitCheckpoint(source: name, generation: generation, checkpoint: UUID().uuidString, observations: [], scanID: "", scanStartedAt: "", scope: [], completionEvidence: "", blobHandles: [:], live: false)
    }
    public func prepare(_ command: JSON) async throws -> Prepared {
        let action = command["action"]
        guard action["op"].string == "pdf.annotate" else { throw SourceFailure.invalid }
        let data = try queryDocument(action["args"])
        _ = try document(action["args"], data: data)
        let digest = SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
        return Prepared(nativeState: .object(["digest": .string(digest)]), fingerprint: command["request_hash"].string, action: action)
    }
    private func document(_ args: JSON, data: Data) throws -> PDFDocument {
        guard let document = PDFDocument(data: data), !document.isLocked, document.pageCount <= 1000,
              !args["annotations"].array.isEmpty, args["annotations"].array.count <= 100 else { throw SourceFailure.invalid }
        return document
    }
    public func dispatch(_ prepared: Prepared) async throws -> JSON {
        guard artifacts.count < 128 else { throw SourceFailure.unavailable }
        let args = prepared.action["args"]
        let input = try queryDocument(args)
        let digest = SHA256.hash(data: input).map { String(format: "%02x", $0) }.joined()
        guard digest == prepared.nativeState["digest"].string else { throw SourceFailure.conflict }
        let document = try document(args, data: input)
        for item in args["annotations"].array {
            try Task.checkCancellation()
            let number = item["page"].number
            guard number >= 1, number.rounded() == number, number <= Double(document.pageCount), let page = document.page(at: Int(number) - 1) else { throw SourceFailure.invalid }
            let kind = item["type"].string
            let rect = CGRect(x: item["x"].number, y: item["y"].number, width: item["width"].number, height: item["height"].number)
            guard [rect.minX, rect.minY, rect.width, rect.height].allSatisfy({ $0.isFinite }), rect.width > 0, rect.height > 0, page.bounds(for: .mediaBox).contains(rect), ["note", "highlight"].contains(kind), item["text"].string.utf8.count <= 4096 else { throw SourceFailure.invalid }
            let annotation = PDFAnnotation(bounds: rect, forType: kind == "note" ? .text : .highlight, withProperties: nil)
            annotation.contents = item["text"].string
            if kind == "highlight" { annotation.quadrilateralPoints = [NSValue(point: CGPoint(x: 0, y: rect.height)), NSValue(point: CGPoint(x: rect.width, y: rect.height)), NSValue(point: .zero), NSValue(point: CGPoint(x: rect.width, y: 0))] }
            page.addAnnotation(annotation)
        }
        guard let bytes = document.dataRepresentation(), bytes.count <= 16 << 20, let output = PDFDocument(data: bytes) else { throw SourceFailure.corrupt }
        for item in args["annotations"].array {
            guard let page = output.page(at: Int(item["page"].number) - 1), page.annotations.contains(where: { $0.contents == item["text"].string && $0.type == (item["type"].string == "note" ? "Text" : "Highlight") }) else { throw SourceFailure.corrupt }
        }
        let blob = try writer.stage(bytes)
        let key = UUID().uuidString
        let metadata: JSON = .object(["blob": blob.reference, "annotation_count": .number(Double(args["annotations"].array.count))])
        artifacts[key] = (metadata, blob)
        return .object(["native_key": .string(key), "annotation_count": .number(Double(args["annotations"].array.count)), "observed": .bool(true), "disposition": .string("locally_observed")])
    }
    public func resolve(_ command: String, evidence: JSON) async -> JSON {
        guard artifacts[evidence["native_key"].string] != nil else { return .null }
        return evidence
    }
    public func captureCommand(_ evidence: JSON, into sink: any ObservationSink) async throws {
        let key = evidence["native_key"].string
        guard let artifact = artifacts[key] else { throw SourceFailure.unavailable }
        let row: JSON = .object(["kind": .string("upsert"), "collection": .string("pdf.artifact"), "native_key": .string(key), "observed_at": .string(timestamp()), "data": artifact.0])
        try await sink.emitCheckpoint(source: name, generation: generation, checkpoint: UUID().uuidString, observations: [row], scanID: "", scanStartedAt: "", scope: [], completionEvidence: "", blobHandles: [artifact.1.digest: "pdf:" + artifact.1.digest], live: true)
    }
    public func readBlob(_ handle: String, offset: Int64, length: Int) throws -> JSON {
        let digest = String(handle.dropFirst(4))
        guard handle.hasPrefix("pdf:"), digest.count == 64, digest.allSatisfy({ "0123456789abcdef".contains($0) }), offset >= 0, length > 0, length <= 131072 else { throw SourceFailure.invalid }
        let url = writer.directory.appendingPathComponent(digest)
        let size = try url.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0
        guard offset <= Int64(size) else { throw SourceFailure.invalid }
        let file = try FileHandle(forReadingFrom: url)
        defer { try? file.close() }
        try file.seek(toOffset: UInt64(offset))
        let bytes = try file.read(upToCount: length) ?? Data()
        return .object(["t": .string("BlobChunk"), "handle": .string(handle), "offset": .number(Double(offset)), "digest": .string(digest), "bytes": .string(bytes.base64EncodedString()), "eof": .bool(offset + Int64(bytes.count) >= Int64(size))])
    }
}
