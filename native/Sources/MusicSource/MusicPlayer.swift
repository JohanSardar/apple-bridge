import Foundation
import AppKit
import Carbon
import SourceSupport

@MainActor
public final class MusicPlayer: Source {
    public nonisolated let name = "music"
    private let operations = ["music.play", "music.pause", "music.next", "music.previous", "music.set_volume", "music.play_item", "music.add_to_playlist", "music.favorite"]
    var sink: (any ObservationSink)?
    var generation = UUID().uuidString
    var publishing = false
    public init() {}
    private func automationStatus() -> OSStatus {
        let target = NSAppleEventDescriptor(bundleIdentifier: "com.apple.Music")
        return AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), false)
    }
    public func open() async -> SourceReadiness {
        let status = automationStatus()
        return SourceReadiness(state: status == noErr ? "ready" : status == -600 ? "degraded" : "permission_missing", coverage: .object(["fields": .string("partial"), "missing": .array([.string("Library capture bounded to 500 tracks, 50 playlists and 1000 memberships")])]), capabilities: status == noErr ? operations : [], reason: status == noErr ? "" : status == -600 ? "Music is not running" : "Automation: dev.applebridge.helper → com.apple.Music")
    }
    public func capture(_ resume: JSON, into sink: any ObservationSink) async throws {
        self.sink = sink
        if !resume["generation"].string.isEmpty { generation = resume["generation"].string }
        var previous: [JSON]?
        while !Task.isCancelled {
            if publishing { try await Task.sleep(for: .seconds(1)); continue }
            let rows = try library()
            if rows != previous { try await publishLibrary(rows); previous = rows }
            try await Task.sleep(for: .seconds(60))
        }
    }
    func script(_ body: String) throws -> NSAppleEventDescriptor {
        let status = automationStatus()
        guard status == noErr else { if status == -600 { throw SourceFailure.unavailable }; throw SourceFailure.permissionMissing }
        var error: NSDictionary?
        guard let script = NSAppleScript(source: "with timeout of 5 seconds\ntell application id \"com.apple.Music\"\n" + body + "\nend tell\nend timeout") else { throw SourceFailure.invalid }
        let result = script.executeAndReturnError(&error)
        guard error == nil else { throw SourceFailure.unavailable }
        return result
    }
    public func nowPlaying() throws -> JSON {
        let result = try script("set s to player state as string\nset v to sound volume\nif player state is stopped then return {s, v, 0, \"\", \"\", \"\", \"\"}\nreturn {s, v, player position, persistent ID of current track, name of current track, artist of current track, album of current track}")
        guard result.numberOfItems == 7 else { throw SourceFailure.corrupt }
        return .object(["state": .string(result.atIndex(1)?.stringValue ?? ""), "volume": .number(Double(result.atIndex(2)?.int32Value ?? 0)), "position": .number(result.atIndex(3)?.doubleValue ?? 0), "id": .string(result.atIndex(4)?.stringValue?.uppercased() ?? ""), "track": .string(result.atIndex(5)?.stringValue ?? ""), "artist": .string(result.atIndex(6)?.stringValue ?? ""), "album": .string(result.atIndex(7)?.stringValue ?? "")])
    }
    func action(_ action: JSON) throws -> String {
        let args = action["args"]
        switch action["op"].string {
        case "music.play", "music.pause", "music.next", "music.previous":
            guard args == .object([:]) else { throw SourceFailure.invalid }
            return ["music.play": "play", "music.pause": "pause", "music.next": "next track", "music.previous": "previous track"][action["op"].string]!
        case "music.set_volume":
            let volume = args["volume"].number
            guard args.object.count == 1, volume >= 0, volume <= 100, volume.rounded() == volume, args["volume"] == .number(volume) else { throw SourceFailure.invalid }
            return "set sound volume to " + String(Int(volume))
        case "music.favorite", "music.add_to_playlist":
            let id = args["id"].string
            guard id.count == 16, id.allSatisfy({ $0.isHexDigit && $0.isASCII }), args.object.count == 2 else { throw SourceFailure.invalid }
            let target = "(first track of library playlist 1 whose persistent ID is \"" + id + "\")"
            if action["op"].string == "music.favorite" {
                guard case .bool(let value) = args["favorite"] else { throw SourceFailure.invalid }
                return "set favorited of " + target + " to " + (value ? "true" : "false")
            }
            let playlist = args["playlist_id"].string
            guard playlist.count == 16, playlist.allSatisfy({ $0.isHexDigit && $0.isASCII }) else { throw SourceFailure.invalid }
            return "duplicate " + target + " to (first user playlist whose persistent ID is \"" + playlist + "\")"
        case "music.play_item":
            let id = args["id"].string
            guard args.object.count == 1, id.count == 16, id.allSatisfy({ $0.isHexDigit && $0.isASCII }) else { throw SourceFailure.invalid }
            return "play (first track of library playlist 1 whose persistent ID is \"" + id + "\")"
        default: throw SourceFailure.unsupported
        }
    }
    public func prepare(_ command: JSON) async throws -> Prepared {
        guard !publishing else { throw SourceFailure.conflict }
        _ = try action(command["action"])
        if ["music.favorite", "music.add_to_playlist"].contains(command["action"]["op"].string) { _ = try library() }
        let before = try nowPlaying()
        return Prepared(nativeState: .object(["id": before["id"], "state": before["state"], "volume": before["volume"]]), fingerprint: command["request_hash"].string, action: command["action"])
    }
    public func dispatch(_ prepared: Prepared) async throws -> JSON {
        guard !publishing else { throw SourceFailure.conflict }
        _ = try script(action(prepared.action))
        let after = try nowPlaying()
        let op = prepared.action["op"].string
        var libraryVerified: Bool?
        if op == "music.favorite" || op == "music.add_to_playlist" {
            let rows = try library()
            let args = prepared.action["args"]
            if op == "music.favorite" { libraryVerified = rows.contains { $0["collection"].string == "music.library_item" && $0["native_key"] == args["id"] && $0["data"]["favorite"] == args["favorite"] } }
            else { libraryVerified = rows.contains { $0["collection"].string == "music.playlist" && $0["native_key"] == args["playlist_id"] && $0["data"]["items"].array.contains { $0["$ref"]["native_key"] == args["id"] } } }
            try await publishLibrary(rows)
        }
        let verified = libraryVerified ?? (op == "music.play" ? after["state"].string == "playing" : op == "music.pause" ? after["state"].string != "playing" : op == "music.set_volume" ? after["volume"] == prepared.action["args"]["volume"] : op == "music.play_item" ? after["id"] == prepared.action["args"]["id"] : after["id"] != prepared.nativeState["id"])
        var result: [String: JSON] = ["disposition": .string(verified ? "locally_observed" : "indeterminate"), "verified": .bool(verified), "fingerprint": .string(prepared.fingerprint), "player": .object(["state": after["state"], "volume": after["volume"]])]
        if libraryVerified != nil {
            result["observed"] = .bool(verified)
            result["native_key"] = prepared.action["args"][op == "music.favorite" ? "id" : "playlist_id"]
        }
        return .object(result)
    }
    public func resolve(_ command: String, evidence: JSON) async -> JSON { .null }
}
