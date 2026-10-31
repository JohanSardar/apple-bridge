import Foundation
import AppKit
import Carbon
import PDFKit
import SourceSupport

@MainActor
extension MacSource {
    func exportIWork(_ args: JSON) throws -> JSON {
        let applications = ["pages": "Pages", "numbers": "Numbers", "keynote": "Keynote"]
        guard let application = applications[args["app"].string],
              let bytes = Data(base64Encoded: args["document_blob"].string), !bytes.isEmpty, bytes.count <= 8 * 1024 * 1024,
              ["pdf", "text"].contains(args["format"].string) else { throw SourceFailure.invalid }
        guard args["format"].string != "text" || application == "Pages" else { throw SourceFailure.unsupported }
        let identifier = "com.apple.iWork." + application
        guard NSWorkspace.shared.urlForApplication(withBundleIdentifier: identifier) != nil else { throw SourceFailure.unavailable }
        let target = NSAppleEventDescriptor(bundleIdentifier: identifier)
        guard AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), false) == noErr else { throw SourceFailure.permissionMissing }
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("[ab-test] iWork " + UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        defer { try? FileManager.default.removeItem(at: directory) }
        let plain = application == "Pages" && String(data: bytes, encoding: .utf8) != nil && !bytes.starts(with: [0x50, 0x4b])
        let fileExtension = application == "Keynote" ? "key" : args["app"].string
        let input = directory.appendingPathComponent("[ab-test] document." + (plain ? "txt" : fileExtension))
        let output = directory.appendingPathComponent("[ab-test] export." + (args["format"].string == "pdf" ? "pdf" : "txt"))
        try bytes.write(to: input, options: .withoutOverwriting)
        func quote(_ value: String) -> String {
            "\"" + value.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"").replacingOccurrences(of: "\n", with: "\\n").replacingOccurrences(of: "\r", with: "\\r") + "\""
        }
        let format = args["format"].string == "pdf" ? "PDF" : "unformatted text"
        let body = """
        with timeout of 30 seconds
            tell application id \(quote(identifier))
                set bridgeDocument to open POSIX file \(quote(input.path))
                try
                    export bridgeDocument to POSIX file \(quote(output.path)) as \(format)
                on error errorText number errorNumber
                    close bridgeDocument saving no
                    error errorText number errorNumber
                end try
                close bridgeDocument saving no
            end tell
        end timeout
        """
        var error: NSDictionary?
        guard let script = NSAppleScript(source: body) else { throw SourceFailure.invalid }
        _ = script.executeAndReturnError(&error)
        guard error == nil else { throw SourceFailure.unavailable }
        let values = try output.resourceValues(forKeys: [.fileSizeKey, .isRegularFileKey])
        guard values.isRegularFile == true, let size = values.fileSize, size > 0, size <= 16 * 1024 * 1024 else { throw SourceFailure.corrupt }
        let exported = try Data(contentsOf: output)
        if args["format"].string == "pdf" {
            guard let pdf = PDFDocument(data: exported), pdf.pageCount > 0 else { throw SourceFailure.corrupt }
        } else {
            guard String(data: exported, encoding: .utf8) != nil else { throw SourceFailure.corrupt }
        }
        return .object(["app": args["app"], "format": args["format"], "data_base64": .string(exported.base64EncodedString())])
    }
}
