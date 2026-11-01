import Foundation
import SourceSupport

public final class SafariReadScript {
    private let script: NSAppleScript
    public init(source: String) throws {
        guard let script = NSAppleScript(source: source) else { throw SourceFailure.invalid }
        self.script = script
    }
    public func run() throws -> NSAppleEventDescriptor {
        var error: NSDictionary?
        let result = script.executeAndReturnError(&error)
        guard error == nil else { throw SourceFailure.unavailable }
        return result
    }
}
