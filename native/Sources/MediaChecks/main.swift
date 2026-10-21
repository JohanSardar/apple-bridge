import Foundation
import MediaDerivatives
import AVFoundation
import ImageIO
import UniformTypeIdentifiers
import CryptoKit

@main
struct MediaChecks {
    static func main() async throws {
        let args = CommandLine.arguments
        switch args[1] {
        case "thumbhash":
            struct Fixture: Decodable { let width: Int; let height: Int; let rgba: [UInt8]; let hash: String }
            let fixtures = try JSONDecoder().decode([Fixture].self, from: Data(contentsOf: URL(fileURLWithPath: args[2])))
            for fixture in fixtures {
                guard ThumbHash.encode(width: fixture.width, height: fixture.height, rgba: fixture.rgba)?.base64EncodedString() == fixture.hash else { throw CheckFailure.mismatch }
            }
            guard ThumbHash.encode(width: 101, height: 1, rgba: []) == nil, ThumbHash.encode(width: 0, height: 0, rgba: []) == nil else { throw CheckFailure.mismatch }
            print("thumbhash fixtures passed: \(fixtures.count)")
        case "fixtures":
            let directory = URL(fileURLWithPath: args[2], isDirectory: true)
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            let tag = args[3]
            let liveID = UUID().uuidString
            let context = CGContext(data: nil, width: 2400, height: 1200, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
            context.setFillColor(CGColor(red: 0.1, green: 0.6, blue: 0.8, alpha: 1))
            context.fill(CGRect(x: 0, y: 0, width: 2400, height: 1200))
            context.setFillColor(CGColor(red: 1, green: 0.2, blue: 0, alpha: 1))
            context.fill(CGRect(x: 0, y: 0, width: 800, height: 1200))
            let photo = directory.appendingPathComponent("ab-" + tag + ".heic")
            let destination = CGImageDestinationCreateWithURL(photo as CFURL, UTType.heic.identifier as CFString, 1, nil)!
            CGImageDestinationAddImage(destination, context.makeImage()!, [kCGImagePropertyOrientation: 6, kCGImagePropertyTIFFDictionary: [kCGImagePropertyTIFFDocumentName: tag], kCGImagePropertyMakerAppleDictionary: ["17": liveID]] as CFDictionary)
            guard CGImageDestinationFinalize(destination) else { throw CheckFailure.mismatch }
            try await movie(directory.appendingPathComponent("ab-" + tag + ".mov"), small: tag == "sim", identifier: liveID)
            let format = AVAudioFormat(standardFormatWithSampleRate: 16000, channels: 1)!
            let audio = directory.appendingPathComponent("ab-" + tag + ".caf")
            let file = try AVAudioFile(forWriting: audio, settings: format.settings)
            let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 16000)!
            buffer.frameLength = 16000
            for i in 0..<16000 { buffer.floatChannelData![0][i] = tag == "sim" ? 0 : Float(sin(Double(i) * 2 * .pi * 440 / 16000) * Double(i) / 32000) }
            try file.write(from: buffer)
            print("synthetic HEIC, MOV and CAF created")
        case "derive":
            let file = URL(fileURLWithPath: args[2])
            let digest = SHA256.hash(data: try Data(contentsOf: file)).map { String(format: "%02x", $0) }.joined()
            let result = try await MediaDerivatives.make(file, digest: digest)
            print(String(data: try JSONEncoder().encode(result), encoding: .utf8)!)
        case "inspect":
            let file = URL(fileURLWithPath: args[2])
            if let source = CGImageSourceCreateWithURL(file as CFURL, nil), let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [String: Any] {
                let value: [String: Any] = ["type": CGImageSourceGetType(source)! as String, "width": properties[kCGImagePropertyPixelWidth as String]!, "height": properties[kCGImagePropertyPixelHeight as String]!]
                print(String(data: try JSONSerialization.data(withJSONObject: value), encoding: .utf8)!)
            } else {
                let asset = AVURLAsset(url: file)
                var value: [String: Any] = ["duration_ms": try await asset.load(.duration).seconds * 1000]
                for (name, kind) in [("video", AVMediaType.video), ("audio", AVMediaType.audio)] {
                    if let track = try await asset.loadTracks(withMediaType: kind).first {
                        let formats = try await track.load(.formatDescriptions)
                        value[name] = formats.map { format in
                            let code = CMFormatDescriptionGetMediaSubType(format)
                            return String(bytes: [UInt8(truncatingIfNeeded: code >> 24), UInt8(truncatingIfNeeded: code >> 16), UInt8(truncatingIfNeeded: code >> 8), UInt8(truncatingIfNeeded: code)], encoding: .ascii) ?? "unknown"
                        }
                        if name == "video" {
                            let size = try await track.load(.naturalSize)
                            let transform = try await track.load(.preferredTransform)
                            value["width"] = abs(size.applying(transform).width)
                            value["height"] = abs(size.applying(transform).height)
                        }
                    }
                }
                print(String(data: try JSONSerialization.data(withJSONObject: value), encoding: .utf8)!)
            }
        default: throw CheckFailure.mismatch
        }
    }
    static func movie(_ file: URL, small: Bool, identifier: String) async throws {
        let width = small ? 16 : 320
        let height = small ? 16 : 240
        let writer = try AVAssetWriter(outputURL: file, fileType: .mov)
        let metadata = AVMutableMetadataItem()
        metadata.identifier = .quickTimeMetadataContentIdentifier
        metadata.value = identifier as NSString
        metadata.dataType = kCMMetadataBaseDataType_UTF8 as String
        writer.metadata = [metadata]
        let input = AVAssetWriterInput(mediaType: .video, outputSettings: [AVVideoCodecKey: (small ? AVVideoCodecType.h264 : AVVideoCodecType.hevc), AVVideoWidthKey: width, AVVideoHeightKey: height])
        let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32ARGB, kCVPixelBufferWidthKey as String: width, kCVPixelBufferHeightKey as String: height])
        writer.add(input)
        guard writer.startWriting() else { throw CheckFailure.mismatch }
        writer.startSession(atSourceTime: .zero)
        for i in 0..<12 {
            while !input.isReadyForMoreMediaData { try await Task.sleep(for: .milliseconds(5)) }
            var pixel: CVPixelBuffer?
            guard CVPixelBufferPoolCreatePixelBuffer(nil, adaptor.pixelBufferPool!, &pixel) == kCVReturnSuccess, let pixel else { throw CheckFailure.mismatch }
            CVPixelBufferLockBaseAddress(pixel, [])
            let bytes = CVPixelBufferGetBaseAddress(pixel)!.assumingMemoryBound(to: UInt8.self)
            let stride = CVPixelBufferGetBytesPerRow(pixel)
            for y in 0..<height { for x in 0..<width {
                let offset = y * stride + x * 4
                bytes[offset] = 255
                bytes[offset + 1] = small ? 40 : UInt8(x % 256)
                bytes[offset + 2] = small ? 120 : UInt8(y % 256)
                bytes[offset + 3] = small ? 200 : UInt8(i * 20)
            } }
            CVPixelBufferUnlockBaseAddress(pixel, [])
            guard adaptor.append(pixel, withPresentationTime: CMTime(value: Int64(i), timescale: 12)) else { throw CheckFailure.mismatch }
        }
        input.markAsFinished()
        writer.endSession(atSourceTime: CMTime(value: 1, timescale: 1))
        await writer.finishWriting()
        guard writer.status == .completed else { throw CheckFailure.mismatch }
    }
}

enum CheckFailure: Error { case mismatch }
