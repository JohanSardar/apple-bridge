#if LAB
import Foundation
import CryptoKit
import SourceSupport

private actor MusicQualificationSink: ObservationSink {
    func emitCheckpoint(source: String, generation: String, checkpoint: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {}
    func emit(source: String, generation: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {}
    func sourceState(source: String, state: String, coverage: JSON, capabilities: [String], reason: String) async throws {}
}

extension MusicPlayer {
    private func qualificationSnapshot() throws -> JSON {
        let row = try script("""
        set priorState to player state as string
        try
        if player state is playing then pause
        set trackID to ""
        set playlistID to ""
        set trackIndex to 0
        set positionValue to 0
        try
            set trackID to persistent ID of current track
            set playlistID to persistent ID of current playlist
            set trackIndex to index of current track
            if player state is not stopped then set positionValue to player position
        end try
        return {priorState, sound volume, mute, shuffle enabled, shuffle mode as string, song repeat as string, trackID, playlistID, trackIndex, positionValue}
        on error errorText number errorNumber
            if priorState is "playing" then play
            error errorText number errorNumber
        end try
        """)
        guard row.numberOfItems == 10 else { throw SourceFailure.corrupt }
        return .object(["state": .string(row.atIndex(1)?.stringValue ?? ""), "volume": .number(Double(row.atIndex(2)?.int32Value ?? 0)), "mute": .bool(row.atIndex(3)?.booleanValue ?? false), "shuffle": .bool(row.atIndex(4)?.booleanValue ?? false), "shuffle_mode": .string(row.atIndex(5)?.stringValue ?? ""), "repeat": .string(row.atIndex(6)?.stringValue ?? ""), "track": .string(row.atIndex(7)?.stringValue ?? ""), "playlist": .string(row.atIndex(8)?.stringValue ?? ""), "index": .number(Double(row.atIndex(9)?.int32Value ?? 0)), "position": .number(row.atIndex(10)?.doubleValue ?? 0)])
    }

    private func qualificationRestore(_ before: JSON) throws {
        let playlist = before["playlist"].string
        guard ["songs", "albums", "groupings"].contains(before["shuffle_mode"].string), ["off", "one", "all"].contains(before["repeat"].string) else { throw SourceFailure.invalid }
        var context = "stop"
        if !playlist.isEmpty {
            guard playlist.count == 16, playlist.allSatisfy({ $0.isASCII && $0.isHexDigit }), before["track"].string.count == 16, before["track"].string.allSatisfy({ $0.isASCII && $0.isHexDigit }), before["index"].number > 0 else { throw SourceFailure.invalid }
            context = """
            set p to first playlist whose persistent ID is "\(playlist)"
            set t to track \(Int(before["index"].number)) of p
            if persistent ID of t is not "\(before["track"].string)" then error "context changed"
            play t
            pause
            set player position to \(before["position"].number)
            \(before["state"].string == "stopped" ? "stop" : "")
            """
        }
        _ = try script("""
        set mute to true
        set shuffle enabled to false
        \(context)
        set shuffle mode to \(before["shuffle_mode"].string)
        set shuffle enabled to \(before["shuffle"].bool ? "true" : "false")
        set song repeat to \(before["repeat"].string)
        set sound volume to \(Int(before["volume"].number))
        set mute to \(before["mute"].bool ? "true" : "false")
        """)
    }

    public func qualifyPlayback(playlistOnly: Bool) async throws -> JSON {
        let readiness = await open()
        guard readiness.state == "ready" else { return .object(["status": .string("limited"), "reason": .string(readiness.state == "permission_missing" ? "needs Music Automation grant (T79); consent not requested" : "needs Music running; no fixture changes made")]) }
        let before = try qualificationSnapshot()
        guard ["playing", "paused", "stopped"].contains(before["state"].string), (before["state"].string == "stopped" || (!before["track"].string.isEmpty && !before["playlist"].string.isEmpty && before["index"].number > 0)) else {
            if before["state"].string == "playing" { _ = try script("play") }
            return .object(["status": .string("limited"), "reason": .string("needs identifiable active playback context; no fixture changes made")])
        }
        let tag = "[ab-test] " + UUID().uuidString
        var checks: [String: JSON] = [:]
        var after = JSON.null
        var ownsFixtures = false
        do {
            _ = try script("if (count user playlists whose name starts with \"\(tag)\") is not 0 then error \"collision\"")
            ownsFixtures = true
            let fixture = try script("""
            if (count tracks of library playlist 1) is 0 then error "library track required"
            set t to first track of library playlist 1
            set p to make new user playlist with properties {name:"\(tag) source"}
            duplicate t to p
            duplicate t to p
            set destination to make new user playlist with properties {name:"\(tag) destination"}
            return {persistent ID of t, persistent ID of destination, count tracks of p}
            """)
            guard fixture.numberOfItems == 3, let track = fixture.atIndex(1)?.stringValue, let playlist = fixture.atIndex(2)?.stringValue else { throw SourceFailure.corrupt }
            checks["library_playlist_fixture_created"] = .bool(fixture.atIndex(3)?.int32Value == 2)
            sink = MusicQualificationSink()
            defer { sink = nil }
            if !playlistOnly {
                _ = try script("set mute to true\nset sound volume to 0\nset shuffle enabled to false\nset song repeat to off\nplay (first user playlist whose name is \"\(tag) source\")")
            }
            let commands = playlistOnly ? ["add_to_playlist"] : ["pause", "play", "next", "previous", "set_volume", "play_item"]
            for operation in commands {
                var args: [String: JSON] = [:]
                if operation == "set_volume" { args["volume"] = .number(0) }
                if operation == "play_item" || operation == "add_to_playlist" { args["id"] = .string(track) }
                if operation == "add_to_playlist" { args["playlist_id"] = .string(playlist) }
                let prepared = try await prepare(.object(["action": .object(["op": .string("music." + operation), "args": .object(args)]), "request_hash": .string(tag)]))
                let outcome = try await dispatch(prepared)
                checks["command_" + operation] = .bool(outcome["verified"].bool)
            }
        } catch { checks["fixture_and_commands"] = .bool(false) }
        do {
            guard ownsFixtures else { throw SourceFailure.conflict }
            let cleanup = try script("""
            delete (every user playlist whose name starts with "\(tag)")
            return count user playlists whose name starts with "\(tag)"
            """)
            checks["tagged_fixture_deleted"] = .bool(cleanup.int32Value == 0)
        } catch { checks["tagged_fixture_deleted"] = .bool(false) }
        do {
            try qualificationRestore(before)
            checks["restore_applied"] = .bool(true)
            after = try qualificationSnapshot()
            var expected = before.object
            expected["state"] = .string(before["state"].string == "playing" ? "paused" : before["state"].string)
            checks["exact_playback_restore"] = .bool(after == .object(expected))
        } catch { checks["restore_applied"] = .bool(false); checks["exact_playback_restore"] = .bool(false) }
        if before["state"].string == "playing" { _ = try script("play") }
        checks["playback_state_restored"] = .bool((try nowPlaying())["state"] == before["state"])
        let encoder = JSONEncoder(); encoder.outputFormatting = [.sortedKeys]
        func digest(_ value: JSON) throws -> JSON { .string(SHA256.hash(data: try encoder.encode(value)).map { String(format: "%02x", $0) }.joined()) }
        var normalized = after.object; normalized["state"] = before["state"]
        return .object(["status": .string("complete"), "assertions": .object(checks), "before_digest": try digest(before), "after_digest": try digest(.object(normalized))])
    }
}
#endif
