import Foundation
import SourceSupport

public actor SafariSource: Source {
    public nonisolated let name = "safari"
    private let tabs: SafariTabs
    private let directory: URL
    public init(directory: URL = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Safari"), tabs: SafariTabs = SafariTabs()) { self.directory = directory; self.tabs = tabs }
    public func open() async -> SourceReadiness {
        guard FileManager.default.fileExists(atPath: directory.path) else {
            return SourceReadiness(state: "unavailable", coverage: .object(["fields": .string("partial")]), reason: "Safari data directory unavailable")
        }
        let permitted = await tabs.permitted()
        let missing: [JSON] = [.string("CloudTabs schema availability varies")] + (permitted ? [] : [.string("Automation dev.applebridge.helper → com.apple.Safari")])
        let readable = ["History.db", "Bookmarks.plist", "CloudTabs.db"].contains { FileManager.default.isReadableFile(atPath: directory.appendingPathComponent($0).path) }
        return SourceReadiness(state: readable ? "ready" : "permission_missing", coverage: .object(["fields": .string("partial"), "missing": .array(missing)]), capabilities: [], reason: readable ? (permitted ? "" : "Automation dev.applebridge.helper → com.apple.Safari") : "Full Disk Access required")
    }
    public func liveTabs() async throws -> [JSON] { try await tabs.tabs() }
}
