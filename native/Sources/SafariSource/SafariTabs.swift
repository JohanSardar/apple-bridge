import Foundation
import AppKit
import Carbon
import SourceSupport

@MainActor
public final class SafariTabs {
    private let execute: (@Sendable (String) throws -> NSAppleEventDescriptor)?
    public nonisolated init(execute: (@Sendable (String) throws -> NSAppleEventDescriptor)? = nil) { self.execute = execute }
    private var permission: (Bool, ContinuousClock.Instant)?
    public func permitted(fresh: Bool = false) -> Bool {
        if execute != nil { return true }
        if !fresh, let permission, permission.1.duration(to: .now) < .seconds(300) { return permission.0 }
        let target = NSAppleEventDescriptor(bundleIdentifier: "com.apple.Safari")
        let allowed = AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), false) == noErr
        permission = (allowed, .now)
        return allowed
    }
    private func script(_ body: String) throws -> NSAppleEventDescriptor {
        if let execute { return try execute(body) }
        guard permitted(), permitted(fresh: true) else { throw SourceFailure.permissionMissing }
        var error: NSDictionary?
        guard let script = NSAppleScript(source: "with timeout of 5 seconds\ntell application id \"com.apple.Safari\"\n" + body + "\nend tell\nend timeout") else { throw SourceFailure.invalid }
        let result = script.executeAndReturnError(&error)
        guard error == nil else { throw SourceFailure.unavailable }
        return result
    }
    public func tabs() throws -> [JSON] {
        let result = try script("set rows to {}\nrepeat with w in windows\nset n to 0\nrepeat with t in tabs of w\nset n to n + 1\nset end of rows to {id of w, n, name of t, URL of t, (index of w is 1 and current tab of w is t)}\nend repeat\nend repeat\nreturn rows")
        var rows: [JSON] = []
        for i in 1..<(result.numberOfItems + 1) {
            guard let row = result.atIndex(i), row.numberOfItems == 5 else { throw SourceFailure.corrupt }
            let window = row.atIndex(1)!.int32Value
            let index = row.atIndex(2)!.int32Value
            let key = "\(window):\(index)"
            rows.append(.object(["kind": .string("upsert"), "collection": .string("safari.tab"), "native_key": .string(key), "scoped_alias": .string("safari/safari.tab/" + key), "observed_at": .string(timestamp()), "data": .object(["window": .number(Double(window)), "index": .number(Double(index)), "title": .string(row.atIndex(3)?.stringValue ?? ""), "url": .string(row.atIndex(4)?.stringValue ?? ""), "active": .bool(row.atIndex(5)?.booleanValue ?? false)])]))
        }
        return rows
    }
}
