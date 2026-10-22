#if LAB
import Foundation
import SourceSupport

@MainActor
public enum NotesParityProbe {
    private struct AppleEventFailure: Error { let code: Int }
    private static func script(_ body: String) throws -> String {
        var error: NSDictionary?
        guard let source = NSAppleScript(source: "with timeout of 5 seconds\ntell application id \"com.apple.Notes\"\n" + body + "\nend tell\nend timeout") else { throw SourceFailure.invalid }
        let result = source.executeAndReturnError(&error)
        if let error { throw AppleEventFailure(code: error["NSAppleScriptErrorNumber"] as? Int ?? 0) }
        return result.stringValue ?? ""
    }
    public static func run() async -> JSON {
        let tag = "[ab-test notes-parity-" + UUID().uuidString + "]"
        let store = NotesStore(path: FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Group Containers/group.com.apple.notes/NoteStore.sqlite").path)
        var folderID = "", folderCreated = false, effects = 0
        var proofs: [String: JSON] = [:]
        @MainActor func snapshot(_ id: String) async throws -> JSON {
            for _ in 0..<50 {
                if let note = try store.read().observations.first(where: { $0["scoped_alias"].string == id }), note["data"]["content_status"].string == "complete" { return note }
                try await Task.sleep(for: .milliseconds(100))
            }
            throw SourceFailure.unavailable
        }
        @MainActor func create(_ name: String, html: String) throws -> String {
            let title = tag + " " + name
            let content = "<div>" + title + "</div>" + html
            let id = try script("return id of (make new note at folder id " + NotesScript.quote(folderID) + " with properties {body:" + NotesScript.quote(content) + "})")
            effects += 1
            return id
        }
        @MainActor func replace(_ value: JSON, token: String, with replacement: String) -> JSON {
            switch value {
            case .string(let text): return .string(text.replacingOccurrences(of: token, with: replacement))
            case .array(let values): return .array(values.map { replace($0, token: token, with: replacement) })
            case .object(let values): return .object(values.mapValues { replace($0, token: token, with: replacement) })
            default: return value
            }
        }
        @MainActor func blocks(_ value: JSON) -> [JSON] {
            value.array.flatMap { [$0] + blocks($0["children"]) }
        }
        @MainActor func write(_ id: String, html: String) throws {
            let name = try script("return name of note id " + NotesScript.quote(id))
            guard name.hasPrefix(tag) else { throw SourceFailure.conflict }
            _ = try script("set body of note id " + NotesScript.quote(id) + " to " + NotesScript.quote(html))
            effects += 1
        }
        do {
            let result = try script("set matches to every folder whose name is \"Bridge Test\"\nset made to false\nif (count matches) is 0 then\nset target to make new folder with properties {name:\"Bridge Test\"}\nset made to true\nelse\nset target to item 1 of matches\nend if\nreturn (id of target) & linefeed & (made as string)")
            let parts = result.components(separatedBy: "\n")
            guard parts.count == 2, !parts[0].isEmpty else { throw SourceFailure.unavailable }
            folderID = parts[0]; folderCreated = parts[1] == "true"
            let simple = try create("simple", html: "<div><b>bold-token</b> <i>italic-token</i> <a href=\"https://example.org/\">link-token</a></div>")
            let before = try await snapshot(simple)
            let html = before["data"]["html"].string
            guard html.contains("bold-token") else { throw SourceFailure.unsupported }
            try write(simple, html: html.replacingOccurrences(of: "bold-token", with: "bold-token edited"))
            var after = try await snapshot(simple)
            for _ in 0..<50 where !after["data"]["plain_text"].string.contains("bold-token edited") {
                try await Task.sleep(for: .milliseconds(100)); after = try await snapshot(simple)
            }
            let expected = replace(before["data"]["blocks"], token: "bold-token", with: "bold-token edited")
            let observed = after["data"]["plain_text"].string == before["data"]["plain_text"].string.replacingOccurrences(of: "bold-token", with: "bold-token edited")
            let exact = observed && after["data"]["blocks"] == expected
            let lossless = before["data"]["round_trip_status"].string == "unproven" && after["data"]["round_trip_status"].string == "unproven"
            let reasons = Set(blocks(before["data"]["blocks"]).compactMap { $0["unsupported"] == .null ? nil : $0["unsupported"].string })
            proofs["simple_rich"] = .object(["state": .string(exact && lossless ? "candidate" : "limited"), "reason": .string(!exact ? "html_changed_structure" : !lossless ? "decoder_roundtrip_unqualified" : "production_command_proof_required"), "effect_observed": .bool(observed), "structure_preserved": .bool(exact), "unknown_attributes": .array(reasons.sorted().map(JSON.string))])
            let checklistHTML = "<ul><li><input type=\"checkbox\">unchecked-token</li><li><input type=\"checkbox\" checked>checked-token</li></ul>"
            let checklist = try create("checklist", html: checklistHTML)
            let checkBefore = try await snapshot(checklist)
            try write(checklist, html: "<div>" + tag + " checklist</div><ul><li><input type=\"checkbox\" checked>unchecked-token</li><li><input type=\"checkbox\">checked-token</li></ul>")
            var checkAfter = try await snapshot(checklist)
            for _ in 0..<50 where checkAfter["data"]["native_fingerprint"] == checkBefore["data"]["native_fingerprint"] {
                try await Task.sleep(for: .milliseconds(100)); checkAfter = try await snapshot(checklist)
            }
            let initialItems = blocks(checkBefore["data"]["blocks"]).filter { $0["type"].string == "checklist_item" }
            let finalItems = blocks(checkAfter["data"]["blocks"]).filter { $0["type"].string == "checklist_item" }
            let identities = initialItems.map { $0["checklist_id"] }
            let retained = identities.count == 2 && identities == finalItems.map { $0["checklist_id"] }
            let toggled = retained && finalItems[0]["checked"].bool && !finalItems[1]["checked"].bool
            proofs["checklist"] = .object(["state": .string(toggled ? "candidate" : "limited"), "reason": .string(initialItems.count != 2 ? "public_html_did_not_create_native_checklists" : !retained ? "checklist_identity_not_preserved" : !toggled ? "checklist_state_not_toggled" : "production_command_proof_required"), "before_count": .number(Double(initialItems.count)), "after_count": .number(Double(finalItems.count)), "identities_preserved": .bool(retained)])
        } catch {
            let reason = (error as? AppleEventFailure).map { "apple_event_" + String($0.code) } ?? String(describing: error)
            proofs["failure"] = .object(["state": .string("limited"), "reason": .string(reason)])
            for name in ["simple_rich", "checklist"] where proofs[name] == nil {
                proofs[name] = .object(["state": .string("limited"), "reason": .string(reason)])
            }
        }
        var cleanup = true
        do {
            let current = try store.read()
            let folder = current.observations.first { $0["scoped_alias"].string == folderID }
            let key = folder?["native_key"].string ?? ""
            let notes = current.observations.filter { $0["collection"].string == "notes.note" && $0["data"]["title"].string.hasPrefix(tag) && $0["data"]["folder_id"]["$ref"]["native_key"].string == key }
            for note in notes {
                let id = note["scoped_alias"].string
                let name = try script("return name of note id " + NotesScript.quote(id))
                guard name.hasPrefix(tag) else { throw SourceFailure.conflict }
                _ = try script("delete note id " + NotesScript.quote(id))
            }
            for attempt in 0..<50 {
                let remaining = try notes.contains { try !["gone", "marked", "trashed"].contains(store.deletionState($0["native_key"].string, collection: "notes.note")) }
                if !remaining { break }
                if attempt == 49 { cleanup = false }
                try await Task.sleep(for: .milliseconds(100))
            }
        } catch { cleanup = false }
        return .object(["state": .string(cleanup ? "verified" : "limited"), "os": .string(ProcessInfo.processInfo.operatingSystemVersionString), "proofs": .object(proofs), "mutation_calls": .number(Double(effects)), "cleanup": .string(cleanup ? "complete" : "unverified"), "test_container_created": .bool(folderCreated), "tag": .string(tag)])
    }
}
#endif
