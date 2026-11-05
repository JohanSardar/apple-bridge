import Foundation
import ImageIO
import CoreGraphics
import PDFKit
import SourceSupport

public struct FilesSource: Source {
    public let name = "files"
    public init() {}
    public func ubiquity(_ paths: [String]) -> [JSON] {
        paths.map { path in
            let url = URL(fileURLWithPath: path)
            let status = (try? url.resourceValues(forKeys: [.ubiquitousItemDownloadingStatusKey]))?.ubiquitousItemDownloadingStatus?.rawValue ?? "local"
            return .object(["path": .string(path), "status": .string(status)])
        }
    }
    private func cloudState(_ url: URL) throws -> JSON {
        let values = try url.resourceValues(forKeys: [.isUbiquitousItemKey, .ubiquitousItemDownloadingStatusKey, .ubiquitousItemIsUploadedKey, .ubiquitousItemUploadingErrorKey])
        return .object(["ubiquitous": .bool(values.isUbiquitousItem == true), "status": .string(values.ubiquitousItemDownloadingStatus?.rawValue ?? ""), "uploaded": .bool(values.ubiquitousItemIsUploaded == true), "upload_error": .bool(values.ubiquitousItemUploadingError != nil)])
    }
    private func stage(_ url: URL, writer: BlobWriter) throws -> JSON {
        var staged: StagedBlob?
        var failure: Error?
        var coordinationError: NSError?
        NSFileCoordinator().coordinate(readingItemAt: url, options: [], error: &coordinationError) { coordinated in
            do { staged = try writer.stage(coordinated) } catch { failure = error }
        }
        if let failure { throw failure }
        if let coordinationError { throw coordinationError }
        guard let blob = staged else { throw SourceFailure.unavailable }
        return .object(["digest": .string(blob.digest), "size": .number(Double(blob.size)), "file": .string(blob.file.path)])
    }
    public func primitive(_ operation: String, args: JSON) async throws -> JSON {
        var url = URL(fileURLWithPath: args["path"].string)
        let fm = FileManager.default
        switch operation {
        case "files.cloud_state":
            url.removeAllCachedResourceValues()
            return try cloudState(url)
        case "files.start_download":
            try fm.startDownloadingUbiquitousItem(at: url)
            return .object([:])
        case "files.evict_item":
            try fm.evictUbiquitousItem(at: url)
            return .object([:])
        case "files.trash_item":
            var trashed: NSURL?
            try fm.trashItem(at: url, resultingItemURL: &trashed)
            return .object(["trash_path": .string((trashed as URL?)?.path ?? "")])
        case "files.set_tags":
            var values = URLResourceValues()
            values.tagNames = args["tags"].array.map(\.string)
            try url.setResourceValues(values)
            url.removeAllCachedResourceValues()
            return .object(["tags": .array((try url.resourceValues(forKeys: [.tagNamesKey]).tagNames ?? []).map(JSON.string))])
        case "files.stage":
            return try stage(url, writer: BlobWriter(directory: URL(fileURLWithPath: args["directory"].string)))
        case "files.transform":
            return try transform(args, writer: BlobWriter(directory: URL(fileURLWithPath: args["directory"].string)))
        default:
            throw SourceFailure.unsupported
        }
    }
    private func transform(_ args: JSON, writer: BlobWriter) throws -> JSON {
        let data = try args["inputs"].array.map { input -> Data in
            let url = URL(fileURLWithPath: input.string)
            guard (try url.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 0) <= 64 * 1024 * 1024 else { throw SourceFailure.invalid }
            return try Data(contentsOf: url)
        }
        var outputs: [Data] = []
        let operation = args["operation"].string
        let format = args["format"].string
        var type = "application/pdf"
        if operation == "convert" || operation == "resize" {
            let types = ["png": "public.png", "jpeg": "public.jpeg", "tiff": "public.tiff"]
            guard data.count == 1, let uti = types[format], let source = CGImageSourceCreateWithData(data[0] as CFData, nil), let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else { throw SourceFailure.invalid }
            var result = image
            if operation == "resize" {
                let w = args["width"].number, h = args["height"].number
                guard w >= 1, h >= 1, w <= 8192, h <= 8192, w.rounded() == w, h.rounded() == h else { throw SourceFailure.invalid }
                guard let context = CGContext(data: nil, width: Int(w), height: Int(h), bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { throw SourceFailure.unavailable }
                context.interpolationQuality = .high
                context.draw(image, in: CGRect(x: 0, y: 0, width: w, height: h))
                guard let resized = context.makeImage() else { throw SourceFailure.unavailable }
                result = resized
            }
            let bytes = NSMutableData()
            guard let destination = CGImageDestinationCreateWithData(bytes, uti as CFString, 1, nil) else { throw SourceFailure.unavailable }
            CGImageDestinationAddImage(destination, result, nil)
            guard CGImageDestinationFinalize(destination), let check = CGImageSourceCreateWithData(bytes, nil), CGImageSourceGetType(check) as String? == uti else { throw SourceFailure.corrupt }
            outputs = [bytes as Data]
            type = "image/" + format
        } else if operation == "combine" || operation == "split" {
            let documents = try data.map { bytes -> PDFDocument in
                guard let document = PDFDocument(data: bytes), !document.isLocked, document.pageCount > 0, document.pageCount <= 1000 else { throw SourceFailure.invalid }
                return document
            }
            guard operation != "split" || documents.count == 1 else { throw SourceFailure.invalid }
            let combined = PDFDocument()
            for document in documents {
                for index in 0..<document.pageCount {
                    guard let page = document.page(at: index)?.copy() as? PDFPage else { throw SourceFailure.corrupt }
                    if operation == "split" {
                        let single = PDFDocument()
                        single.insert(page, at: 0)
                        guard let bytes = single.dataRepresentation() else { throw SourceFailure.corrupt }
                        outputs.append(bytes)
                    } else { combined.insert(page, at: combined.pageCount) }
                }
            }
            if operation == "combine" {
                guard let bytes = combined.dataRepresentation() else { throw SourceFailure.corrupt }
                outputs = [bytes]
            }
            guard outputs.allSatisfy({ PDFDocument(data: $0)?.pageCount ?? 0 > 0 }) else { throw SourceFailure.corrupt }
        } else { throw SourceFailure.invalid }
        let references = try outputs.map { bytes -> JSON in
            let blob = try writer.stage(bytes)
            return .object(["digest": .string(blob.digest), "size": .number(Double(blob.size)), "file": .string(blob.file.path), "media_type": .string(type)])
        }
        return .object(["outputs": .array(references)])
    }
}
