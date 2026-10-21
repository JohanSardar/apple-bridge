import Foundation
import CryptoKit
import ImageIO
import UniformTypeIdentifiers
import MessagesSource
import MediaDerivatives
import SourceSupport

struct MediaPriorityChecks {
    static func run() async throws {
        let tag = "ab-test-media-" + UUID().uuidString
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(tag)
        let attachmentRoot = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/Attachments/" + tag)
        let imageURL = attachmentRoot.appendingPathComponent(tag + ".png")
        let path = root.appendingPathComponent("chat.db").path
        var variants: URL?
        var attachmentCreated = false
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
        defer {
            let identity = SHA256.hash(data: Data(path.utf8)).map { String(format: "%02x", $0) }.joined()
            let baseline = NSHomeDirectory() + "/Library/Application Support/AppleBridge/messages/" + identity + ".sqlite"
            for suffix in ["", "-wal", "-shm"] { try? FileManager.default.removeItem(atPath: baseline + suffix) }
            if let variants { try? FileManager.default.removeItem(at: variants) }
            if attachmentCreated {
                try? FileManager.default.removeItem(at: imageURL)
                if (try? FileManager.default.contentsOfDirectory(atPath: attachmentRoot.path).isEmpty) == true { try? FileManager.default.removeItem(at: attachmentRoot) }
            }
            try? FileManager.default.removeItem(at: root)
        }
        try FileManager.default.createDirectory(at: attachmentRoot, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
        attachmentCreated = true
        let context = CGContext(data: nil, width: 800, height: 400, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
        context.setFillColor(CGColor(red: 0.2, green: 0.6, blue: 0.9, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: 800, height: 400))
        let destination = CGImageDestinationCreateWithURL(imageURL as CFURL, UTType.png.identifier as CFString, 1, nil)!
        CGImageDestinationAddImage(destination, context.makeImage()!, [kCGImagePropertyTIFFDictionary: [kCGImagePropertyTIFFDocumentName: tag]] as CFDictionary)
        try MessagesCaptureChecks.require(CGImageDestinationFinalize(destination), "private image fixture")
        let digest = SHA256.hash(data: try Data(contentsOf: imageURL)).map { String(format: "%02x", $0) }.joined()
        let variantRoot = MediaDerivatives.root.appendingPathComponent(digest)
        try MessagesCaptureChecks.require(!FileManager.default.fileExists(atPath: variantRoot.path), "unique media fixture")
        variants = variantRoot
        let db = try SyntheticDatabase(path)
        let filename = imageURL.path.replacingOccurrences(of: "'", with: "''")
        try db.execute("PRAGMA journal_mode=WAL; CREATE TABLE message(guid TEXT,text TEXT); INSERT INTO message VALUES('synthetic-media-message','synthetic'); CREATE TABLE attachment(guid TEXT,filename TEXT,mime_type TEXT,uti TEXT,total_bytes INTEGER,transfer_state INTEGER); INSERT INTO attachment VALUES('synthetic-media-attachment','" + filename + "','image/png','public.png',0,0); CREATE TABLE message_attachment_join(message_id INTEGER,attachment_id INTEGER); INSERT INTO message_attachment_join VALUES(1,1);")
        let foreground = ForegroundActivity.shared.begin()
        var held = true
        defer { if held { ForegroundActivity.shared.end(foreground) } }
        let source = Messages(path: path)
        let sink = CaptureSink()
        let task = Task { try await source.capture(.null, into: sink) }
        defer { task.cancel() }
        try MessagesCaptureChecks.require(await MessagesCaptureChecks.wait {
            await sink.record("synthetic-media-attachment")["blob"]["digest"].string == digest
        }, "capture proceeds while foreground held")
        try await Task.sleep(for: .milliseconds(250))
        let pending = await sink.record("synthetic-media-attachment")
        try MessagesCaptureChecks.require(pending["derivatives"].object.isEmpty, "background media yields to foreground")
        ForegroundActivity.shared.end(foreground)
        held = false
        try MessagesCaptureChecks.require(await MessagesCaptureChecks.wait(10) {
            !(await sink.record("synthetic-media-attachment")["derivatives"]["thumb"]["digest"].string).isEmpty
        }, "media resumes after foreground completes")
        let complete = await sink.record("synthetic-media-attachment")
        let bytes = try await source.readBlob(handle: "media:" + complete["derivatives"]["thumb"]["digest"].string, offset: 0, length: 131072)
        guard let data = Data(base64Encoded: bytes["bytes"].string), let decoded = CGImageSourceCreateWithData(data as CFData, nil), let image = CGImageSourceCreateImageAtIndex(decoded, 0, nil) else { throw SourceFailure.invalid }
        try MessagesCaptureChecks.require(image.width == 400 && image.height == 200, "resumed derivative bytes match image oracle")
        task.cancel()
        _ = try? await task.value
        print("media foreground yield and resume checks passed")
    }
}
