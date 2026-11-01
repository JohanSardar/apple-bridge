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
    private func quote(_ text: String) throws -> String {
        guard !text.contains(where: { $0.isNewline || $0.asciiValue.map({ $0 < 32 }) == true }) else { throw SourceFailure.invalid }
        return "\"" + text.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"") + "\""
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
    public func prepare(_ command: JSON) throws -> Prepared {
        let action = command["action"]
        let op = action["op"].string
        let args = action["args"]
        guard ["safari.open_url", "safari.add_reading_list"].contains(op) else { throw SourceFailure.unsupported }
        guard permitted(fresh: true) else { throw SourceFailure.permissionMissing }
        guard args.object.count == 1,
              args["url"].string.utf8.count <= 2048, let url = URL(string: args["url"].string), ["http", "https"].contains(url.scheme), url.host != nil, url.user == nil else { throw SourceFailure.invalid }
        _ = try quote(args["url"].string)
        return Prepared(nativeState: .null, fingerprint: command["request_hash"].string, action: action)
    }
    public func dispatch(_ prepared: Prepared) async throws -> JSON {
        guard permitted(fresh: true) else { throw SourceFailure.permissionMissing }
        let args = prepared.action["args"]
        let readback: JSON
        switch prepared.action["op"].string {
        case "safari.open_url":
            let url = args["url"].string
            let result = try script("set w to make new document with properties {URL:" + (try quote(url)) + "}\nreturn id of front window")
            let window = result.int32Value
            let key = "\(window):1"
            var matched: JSON?
            for _ in 0..<50 {
                matched = try tabs().first(where: { $0["native_key"].string == key && $0["data"]["url"].string == url })
                if matched != nil { break }
                try await Task.sleep(for: .milliseconds(100))
            }
            guard let row = matched else { throw SourceFailure.conflict }
            readback = .object(["tab": .string(key), "url": row["data"]["url"]])
        case "safari.add_reading_list":
            let url = args["url"].string
            _ = try script("add reading list item " + (try quote(url)))
            readback = .object(["url": .string(url)])
        default: throw SourceFailure.unsupported
        }
        return .object(["disposition": .string("locally_observed"), "observed": .bool(true), "fingerprint": .string(prepared.fingerprint), "readback": readback])
    }
}
