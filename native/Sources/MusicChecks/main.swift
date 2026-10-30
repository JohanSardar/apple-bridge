#if LAB
import Foundation
import AppKit
import MusicSource
import SourceSupport

@MainActor
private final class Fixture {
    var state: String
    let original: String
    var volume = 17
    var index = 3
    var track = "AAAAAAAAAAAAAAAA"
    var added = false
    var deleted = false
    var restored = false
    var fail = false
    var scripts: [String] = []
    init(_ state: String, fail: Bool = false) { self.state = state; original = state; self.fail = fail }
    func list(_ values: [NSAppleEventDescriptor]) -> NSAppleEventDescriptor {
        let result = NSAppleEventDescriptor.list()
        for (index, value) in values.enumerated() { result.insert(value, at: index + 1) }
        return result
    }
    func text(_ value: String) -> NSAppleEventDescriptor { NSAppleEventDescriptor(string: value) }
    func run(_ body: String) throws -> NSAppleEventDescriptor {
        scripts.append(body)
        if body.contains("set priorState") {
            let prior = state
            if state == "playing" { state = "paused" }
            return list([text(prior), NSAppleEventDescriptor(int32: Int32(volume)), NSAppleEventDescriptor(boolean: false), NSAppleEventDescriptor(boolean: true), text("songs"), text("all"), text(prior == "stopped" ? "" : track), text(prior == "stopped" ? "" : "BBBBBBBBBBBBBBBB"), NSAppleEventDescriptor(int32: prior == "stopped" ? 0 : Int32(index)), NSAppleEventDescriptor(double: prior == "stopped" ? 0 : 12)])
        }
        if body.contains("set destination to make") {
            if fail { throw SourceFailure.unavailable }
            return list([text("1111111111111111"), text("2222222222222222"), NSAppleEventDescriptor(int32: 2)])
        }
        if body.contains("delete (every user playlist") { deleted = true; return NSAppleEventDescriptor(int32: 0) }
        if body.contains("set shuffle mode to") { restored = true; state = original == "playing" ? "paused" : original; volume = 17; track = "AAAAAAAAAAAAAAAA"; index = 3 }
        else if body.contains("play (first user playlist whose name") { state = "playing"; track = "1111111111111111"; index = 1; volume = 0 }
        else if body == "pause" { state = "paused" }
        else if body == "play" { state = "playing" }
        else if body == "next track" { index += 1 }
        else if body == "previous track" { index -= 1 }
        else if body.hasPrefix("set sound volume to") { volume = 0 }
        else if body.hasPrefix("play (first track") { state = "playing"; track = "1111111111111111" }
        else if body.hasPrefix("duplicate (") { added = true }
        if body == "return index of current track" { return NSAppleEventDescriptor(int32: Int32(index)) }
        if body.contains("set s to player state") { return list([text(state), NSAppleEventDescriptor(int32: Int32(volume)), NSAppleEventDescriptor(double: 12), text(state == "stopped" ? "" : track), text("fixture"), text("artist"), text("album")]) }
        if body.contains("set songs to") {
            let song = list([text("1111111111111111"), text("fixture"), text("artist"), text("album"), NSAppleEventDescriptor(boolean: false)])
            let playlist = list([text("2222222222222222"), text("destination"), list(added ? [text("1111111111111111")] : [])])
            return list([list([song]), list([playlist])])
        }
        return text("")
    }
}

@main
struct Checks {
    @MainActor
    static func main() async throws {
        for state in ["stopped", "paused", "playing"] {
            for playlistOnly in [true, false] {
                let fixture = Fixture(state)
                let result = try await MusicPlayer(execute: fixture.run).qualifyPlayback(playlistOnly: playlistOnly)
                guard result["status"].string == "complete", result["assertions"].object.values.allSatisfy({ $0 == .bool(true) }), result["before_digest"] == result["after_digest"], fixture.deleted, fixture.restored, fixture.state == state else { throw SourceFailure.corrupt }
                guard !fixture.scripts.contains(where: { $0.contains("delete (every track") || $0.contains("POSIX file") }) else { throw SourceFailure.corrupt }
            }
        }
        let failed = Fixture("stopped", fail: true)
        let result = try await MusicPlayer(execute: failed.run).qualifyPlayback(playlistOnly: false)
        guard result["assertions"]["fixture_and_commands"] == .bool(false), failed.deleted, failed.restored else { throw SourceFailure.corrupt }
        print("Music fixture checks passed")
    }
}
#else
@main
struct Checks { static func main() {} }
#endif
