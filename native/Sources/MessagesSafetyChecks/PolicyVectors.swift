import Foundation
import MessagesSource
import SourceSupport

enum PolicyVectors {
    static func origin(_ name: String) -> CommandOrigin {
        switch name {
        case "interactive": return .interactive
        case "owner_self": return .ownerSelf
        default: return .gated
        }
    }

    static func send(_ vector: JSON) -> Bool {
        let own: Set<String>? = vector["own"] == .null ? nil : Set(vector["own"].array.map(\.string))
        let recipients = vector["recipients"].array.map(\.string)
        let expected = vector["expect"].string
        do {
            let ownerSelf = try MessagesSendPolicy.check(origin: origin(vector["origin"].string), text: vector["text"].string, fileCount: Int(vector["files"].number), own: { own }, recipients: { recipients })
            guard expected == "allow" else { return false }
            return vector.object["owner_self"] == nil || vector["owner_self"].bool == ownerSelf
        } catch MessagesSendSafetyFailure.textTagRequired {
            return expected == "text_tag_required"
        } catch SourceFailure.invalid {
            return expected == "invalid"
        } catch { return false }
    }

    static func window(_ vector: JSON) -> [String: Bool] {
        let sent: Date? = vector["sent_at"] == .null ? nil : Date(timeIntervalSince1970: vector["sent_at"].number)
        let window = MessageActionWindow(service: vector["service"].string, isFromMe: vector["is_from_me"].bool, isMessage: vector["is_message"].bool, sentAt: sent, edits: Int(vector["edits"].number))
        var results = [vector["name"].string + "/exists": (window != nil) == vector["exists"].bool]
        for check in vector["checks"].array {
            let at = Date(timeIntervalSince1970: check["at"].number)
            let allowed = window?.allows(edit: check["kind"].string == "edit", at: at) ?? false
            results[vector["name"].string + "/" + check["kind"].string + "@" + String(Int(check["at"].number))] = allowed == check["allowed"].bool
        }
        return results
    }

    static func run(_ vectors: JSON) -> [String: Bool] {
        var results: [String: Bool] = [:]
        for vector in vectors["send"].array { results["send/" + vector["name"].string] = send(vector) }
        for vector in vectors["windows"].array { results.merge(window(vector)) { $1 } }
        return results
    }
}
