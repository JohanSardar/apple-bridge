import Foundation
import SourceSupport
import ShortcutsSource

@main struct ActionsChecks {
    static func main() throws {
        if CommandLine.arguments.last == "real" {
            let result = try ActionCatalog.discover()
            guard !result["items"].array.isEmpty, result["items"].array.count <= 2000, !result["schema_guaranteed"].bool else { throw SourceFailure.corrupt }
            print(result["items"].array.count)
            return
        }
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: root) }
        let app = root.appendingPathComponent("Synthetic.app/Contents")
        let metadata = app.appendingPathComponent("Resources/Metadata.appintents")
        try FileManager.default.createDirectory(at: metadata, withIntermediateDirectories: true)
        let info = try PropertyListSerialization.data(fromPropertyList: ["CFBundleIdentifier": "com.example.synthetic"], format: .xml, options: 0)
        try info.write(to: app.appendingPathComponent("Info.plist"))
        let file = metadata.appendingPathComponent("extract.actionsdata")
        try Data(#"{"actions":{"Export":{"identifier":"Export","title":{"key":"Export"}},"Hidden":{"isDiscoverable":false}}}"#.utf8).write(to: file)
        let result = try ActionCatalog.discover(roots: [root])
        guard result["complete"].bool, result["items"].array.count == 1, result["items"].array[0]["identifier"].string == "Export", !result["schema_guaranteed"].bool else { throw SourceFailure.corrupt }
        try Data("invalid".utf8).write(to: file)
        let corrupt = try ActionCatalog.discover(roots: [root])
        guard !corrupt["complete"].bool, corrupt["items"].array.isEmpty else { throw SourceFailure.corrupt }
        try Data(repeating: 32, count: (4 << 20) + 1).write(to: file)
        let oversized = try ActionCatalog.discover(roots: [root])
        guard !oversized["complete"].bool, oversized["items"].array.isEmpty else { throw SourceFailure.corrupt }
        var actions: [String: Any] = [:]
        for index in 0..<2001 { actions["Action" + String(index)] = ["title": ["key": "Synthetic"]] }
        try JSONSerialization.data(withJSONObject: ["actions": actions]).write(to: file)
        let capped = try ActionCatalog.discover(roots: [root])
        guard !capped["complete"].bool, capped["items"].array.count == 2000 else { throw SourceFailure.corrupt }
        try FileManager.default.removeItem(at: file)
        try FileManager.default.createSymbolicLink(at: file, withDestinationURL: URL(fileURLWithPath: "/etc/passwd"))
        guard try ActionCatalog.discover(roots: [root])["items"].array.isEmpty else { throw SourceFailure.corrupt }
    }
}
