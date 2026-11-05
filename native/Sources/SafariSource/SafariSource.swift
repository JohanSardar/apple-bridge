import Foundation
import SourceSupport

public actor SafariSource: Source {
    public nonisolated let name = "safari"
    private let tabs: SafariTabs
    private let directory: URL
    public init(directory: URL = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Safari"), tabs: SafariTabs = SafariTabs()) { self.directory = directory; self.tabs = tabs }
    public func status() async -> JSON? {
        let readable = ["History.db", "Bookmarks.plist", "CloudTabs.db"].contains { FileManager.default.isReadableFile(atPath: directory.appendingPathComponent($0).path) }
        return .object(["directory": .bool(FileManager.default.fileExists(atPath: directory.path)), "readable": .bool(readable), "automation": .bool(await tabs.permitted())])
    }
    public func liveTabs() async throws -> [JSON] { try await tabs.tabs() }
}
