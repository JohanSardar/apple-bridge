import Foundation
import AppKit
import Carbon
import SourceSupport

@MainActor
public final class MusicPlayer: Source {
    public nonisolated let name = "music"
    private let operations = ["music.play", "music.pause", "music.next", "music.previous", "music.set_volume", "music.play_item"]
    public init() {}
    private func authorized() -> Bool {
        let target = NSAppleEventDescriptor(bundleIdentifier: "com.apple.Music")
        return AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), false) == noErr
    }
    public func open() async -> SourceReadiness {
        SourceReadiness(state: authorized() ? "ready" : "permission_missing", coverage: .object(["fields": .string("partial"), "missing": .array([.string("library capture pending")])]), capabilities: authorized() ? operations : [], reason: "Automation: dev.applebridge.helper → com.apple.Music")
    }
    public func capture(_ resume: JSON, into sink: any ObservationSink) async throws {
        for await _ in AsyncStream<Void>({ _ in }) { try Task.checkCancellation() }
    }
    private func script(_ body: String) throws -> NSAppleEventDescriptor {
        guard authorized() else { throw SourceFailure.permissionMissing }
        var error: NSDictionary?
        guard let script = NSAppleScript(source: "with timeout of 5 seconds\ntell application id \"com.apple.Music\"\n" + body + "\nend tell\nend timeout") else { throw SourceFailure.invalid }
        let result = script.executeAndReturnError(&error)
        guard error == nil else { throw SourceFailure.unavailable }
        return result
    }
    public func nowPlaying() throws -> JSON {
        let result = try script("set s to player state as string\nset v to sound volume\nif player state is stopped then return {s, v, 0, \"\", \"\", \"\", \"\"}\nreturn {s, v, player position, persistent ID of current track, name of current track, artist of current track, album of current track}")
        guard result.numberOfItems == 7 else { throw SourceFailure.corrupt }
        return .object(["state": .string(result.atIndex(1)?.stringValue ?? ""), "volume": .number(Double(result.atIndex(2)?.int32Value ?? 0)), "position": .number(result.atIndex(3)?.doubleValue ?? 0), "id": .string(result.atIndex(4)?.stringValue ?? ""), "track": .string(result.atIndex(5)?.stringValue ?? ""), "artist": .string(result.atIndex(6)?.stringValue ?? ""), "album": .string(result.atIndex(7)?.stringValue ?? "")])
    }
    private func action(_ action: JSON) throws -> String {
        let args = action["args"]
        switch action["op"].string {
        case "music.play", "music.pause", "music.next", "music.previous":
            guard args == .object([:]) else { throw SourceFailure.invalid }
            return ["music.play": "play", "music.pause": "pause", "music.next": "next track", "music.previous": "previous track"][action["op"].string]!
        case "music.set_volume":
            let volume = args["volume"].number
            guard args.object.count == 1, volume >= 0, volume <= 100, volume.rounded() == volume, args["volume"] == .number(volume) else { throw SourceFailure.invalid }
            return "set sound volume to " + String(Int(volume))
        case "music.play_item":
            let id = args["id"].string
            guard args.object.count == 1, id.count == 16, id.allSatisfy({ $0.isHexDigit && $0.isASCII }) else { throw SourceFailure.invalid }
            return "play (first track of library playlist 1 whose persistent ID is \"" + id + "\")"
        default: throw SourceFailure.unsupported
        }
    }
    public func prepare(_ command: JSON) async throws -> Prepared {
        _ = try action(command["action"])
        let before = try nowPlaying()
        return Prepared(nativeState: before, fingerprint: command["request_hash"].string, action: command["action"])
    }
    public func dispatch(_ prepared: Prepared) async throws -> JSON {
        _ = try script(action(prepared.action))
        let after = try nowPlaying()
        let op = prepared.action["op"].string
        let verified = op == "music.play" ? after["state"].string == "playing" : op == "music.pause" ? after["state"].string != "playing" : op == "music.set_volume" ? after["volume"] == prepared.action["args"]["volume"] : op == "music.play_item" ? after["id"] == prepared.action["args"]["id"] : after["id"] != prepared.nativeState["id"]
        return .object(["disposition": .string(verified ? "locally_observed" : "indeterminate"), "verified": .bool(verified), "fingerprint": .string(prepared.fingerprint), "player": after])
    }
    public func resolve(_ command: String, evidence: JSON) async -> JSON { .null }
}
