import Foundation
import SourceSupport

public enum MessagesPins {
    public static func requestedIndex(_ value: JSON) throws -> Int {
        guard case .number(let number) = value, let index = Int(exactly: number), (-1...8).contains(index) else { throw SourceFailure.invalid }
        return index
    }
    public static func identifiers(_ data: Data) throws -> [String] {
        guard let root = try PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any],
              let state = root["pD"] as? [String: Any], let pins = state["pP"] as? [String],
              pins.count <= 9, Set(pins).count == pins.count, pins.allSatisfy({ !$0.isEmpty }) else { throw SourceFailure.corrupt }
        return pins
    }
    public static func read() throws -> [String] {
        let file = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Preferences/com.apple.messages.pinning.plist")
        return try identifiers(Data(contentsOf: file))
    }
    static func index(_ identifier: String, in pins: [String]) -> Int? {
        guard !identifier.isEmpty else { return nil }
        return pins.firstIndex(of: identifier)
    }
}
