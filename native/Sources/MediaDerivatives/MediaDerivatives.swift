import Foundation
import ImageIO
import UniformTypeIdentifiers
import CryptoKit
import IOKit.ps

public struct MediaVariant: Codable, Sendable {
    public let digest: String
    public let size: Int64
    public let mime: String
    public let width: Int?
    public let height: Int?
    public let file: String
    public var handle: String { "media:" + digest }
}

public struct MediaMetadata: Codable, Sendable {
    public var width: Int?
    public var height: Int?
    public var duration_ms: Int?
    public var thumbhash: String?
    public var waveform: [UInt8]?
    public var live: Bool?
}

public struct MediaResult: Codable, Sendable {
    public var awaitingLive: Bool?
    public var derivatives: [String: MediaVariant] = [:]
    public var media = MediaMetadata()
}

public enum MediaDerivatives {
    public static let root = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/AppleBridge/media-derivatives/v1", isDirectory: true)
    public static var paused: Bool {
        if ProcessInfo.processInfo.thermalState == .serious || ProcessInfo.processInfo.thermalState == .critical { return true }
        guard let info = IOPSCopyPowerSourcesInfo()?.takeRetainedValue(), let sources = IOPSCopyPowerSourcesList(info)?.takeRetainedValue() as? [CFTypeRef] else { return false }
        for source in sources {
            guard let values = IOPSGetPowerSourceDescription(info, source)?.takeUnretainedValue() as? [String: Any], values[kIOPSPowerSourceStateKey] as? String == kIOPSBatteryPowerValue,
                  let current = values[kIOPSCurrentCapacityKey] as? Int, let maximum = values[kIOPSMaxCapacityKey] as? Int, maximum > 0 else { continue }
            if Double(current) / Double(maximum) < 0.2 { return true }
        }
        return false
    }
    public static func supports(_ url: URL, uti: String = "", mime: String = "") -> Bool {
        guard let type = UTType(filenameExtension: url.pathExtension) ?? (uti.isEmpty ? nil : UTType(uti)) ?? (mime.isEmpty ? nil : UTType(mimeType: mime)) else { return false }
        return type.conforms(to: .image) || type.conforms(to: .audiovisualContent)
    }
    public static func cached(_ digest: String) -> MediaResult? {
        guard validDigest(digest), let bytes = try? Data(contentsOf: root.appendingPathComponent(digest + "/manifest.json")), let result = try? JSONDecoder().decode(MediaResult.self, from: bytes),
              result.derivatives.values.allSatisfy({ FileManager.default.fileExists(atPath: url($0).path) }) else { return nil }
        return result
    }
    public static func url(_ variant: MediaVariant) -> URL { root.appendingPathComponent(variant.file) }
    public static func make(_ input: URL, digest: String, companions: [URL] = [], sticker: Bool = false, progress: (@Sendable (MediaResult) async throws -> Void)? = nil) async throws -> MediaResult {
        guard validDigest(digest) else { throw MediaFailure.invalid }
        let directory = root.appendingPathComponent(digest, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: root.path)
        var result: MediaResult
        if let source = CGImageSourceCreateWithURL(input as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary), let identifier = CGImageSourceGetType(source), let type = UTType(identifier as String), type.conforms(to: .image) {
            result = try images(source, directory: directory, sticker: sticker)
            if let identifier = contentIdentifier(source) {
                result.awaitingLive = true
                try persist(result, directory: directory)
                try await progress?(result)
                for companion in companions where companion != input {
                    if (try? await movieIdentifier(companion)) == identifier {
                        let clip = try await movie(companion, directory: directory, name: "live")
                        result.derivatives["live"] = clip.derivatives["live"]
                        result.media.live = true
                        result.awaitingLive = false
                        break
                    }
                }
            }
        } else { result = try await audiovisual(input, directory: directory) }
        try Task.checkCancellation()
        try persist(result, directory: directory)
        return result
    }
    static func persist(_ result: MediaResult, directory: URL) throws {
        let bytes = try JSONEncoder().encode(result)
        let manifest = directory.appendingPathComponent("manifest.json")
        try bytes.write(to: manifest, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: manifest.path)
    }
    static func validDigest(_ value: String) -> Bool { value.count == 64 && value.allSatisfy { "0123456789abcdef".contains($0) } }
    static func variant(_ file: URL, mime: String, width: Int? = nil, height: Int? = nil) throws -> MediaVariant {
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: file.path)
        let reader = try FileHandle(forReadingFrom: file)
        defer { try? reader.close() }
        var hash = SHA256()
        var size: Int64 = 0
        while let bytes = try reader.read(upToCount: 128 * 1024), !bytes.isEmpty { try Task.checkCancellation(); hash.update(data: bytes); size += Int64(bytes.count) }
        let digest = hash.finalize().map { String(format: "%02x", $0) }.joined()
        let immutable = file.deletingLastPathComponent().appendingPathComponent(digest + "." + file.pathExtension)
        if FileManager.default.fileExists(atPath: immutable.path) { try FileManager.default.removeItem(at: file) }
        else { try FileManager.default.moveItem(at: file, to: immutable) }
        return MediaVariant(digest: digest, size: size, mime: mime, width: width, height: height, file: immutable.deletingLastPathComponent().lastPathComponent + "/" + immutable.lastPathComponent)
    }
    public static func liveIdentifier(_ url: URL) -> String? {
        guard let source = CGImageSourceCreateWithURL(url as CFURL, [kCGImageSourceShouldCache: false] as CFDictionary) else { return nil }
        return contentIdentifier(source)
    }
    static func contentIdentifier(_ source: CGImageSource) -> String? {
        let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [String: Any]
        let maker = properties?[kCGImagePropertyMakerAppleDictionary as String] as? [String: Any]
        return maker?["17"] as? String
    }
}

enum MediaFailure: Error { case invalid, decode, encode }
