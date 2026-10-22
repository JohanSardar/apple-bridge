import Foundation
import SourceSupport
import Decoders

public enum SendVariantFailure: String, Error {
    case invalidVariant, capabilityUnproven, scheduleDateControlsMissing, scheduleDateControlsUnverified, mentionSuggestionMissing, mentionSuggestionIdentityUnverified, audioConsentPending, audioMessageFlagUnverified, keptReadbackUnavailable, formatMenuUnavailable, effectPickerUnavailable, composerUnavailable, draftPresent, draftChanged, automationDenied, sendReceiptUnknown, effectReadbackUnverified, formattingReadbackUnverified
}
public enum MessagesSendVariants {
    public static let operations = ["messages.send_later", "messages.send_effect", "messages.send_formatted", "messages.send_mention", "messages.send_audio", "messages.keep_audio"]
    public static let styles = ["bold", "italic", "underline", "strikethrough"]
    public static let textEffects = ["big", "small", "shake", "nod", "explode", "ripple", "bloom", "jitter"]
    static let effects = ["slam", "loud", "gentle", "invisible-ink", "echo", "spotlight", "balloons", "confetti", "love", "lasers", "fireworks", "celebration"]
    public static func operation(_ args: JSON) -> String? {
        if args["variant"] == .null { return nil }
        return ["schedule":"messages.send_later", "effect":"messages.send_effect", "formatting":"messages.send_formatted", "mention":"messages.send_mention", "audio":"messages.send_audio"][args["variant"]["kind"].string]
    }
    public static func routeKey(_ route: JSON) -> String {
        BodyDigest.hash(Data((route["kind"].string+"|"+route["chat"].string+"|"+route["handle"].string+"|"+route["service"].string).utf8))
    }
    public static var proofURL: URL { PrivateReport.directory.appendingPathComponent("messages-send-variant-proofs.json") }
    static var osBuild: String { ProcessInfo.processInfo.operatingSystemVersionString }
    static func proof() -> JSON {
        guard let bytes=try? Data(contentsOf:proofURL),let data=try? JSONDecoder().decode(JSON.self,from:bytes),data["os_build"].string==osBuild,data["version"].number==1,data["complete"].bool else { return .null }
        return data
    }
    static func proven(_ operation: String, route: JSON? = nil, args: JSON? = nil) -> Bool {
        proof()["steps"].array.contains { step in
            guard step["operation"].string==operation,step["proven"].bool,route == nil || step["route"].string==routeKey(route!) else { return false }
            guard let args else { return true }
            if operation=="messages.send_effect" { return args["variant"]["effect"]==step["variant"]["effect"] }
            if operation=="messages.send_formatted" {
                let styles=Set(step["variant"]["formatting"].array.flatMap { $0["styles"].array.map(\.string) }),effects=Set(step["variant"]["formatting"].array.map { $0["effect"].string })
                return args["variant"]["formatting"].array.allSatisfy { Set($0["styles"].array.map(\.string)).isSubset(of:styles) && ($0["effect"].string.isEmpty || effects.contains($0["effect"].string)) }
            }
            return false
        }
    }
    static func capabilities() -> [String] {
        var names=operations.filter { proven($0) }
        for step in proof()["steps"].array where step["proven"].bool {
            if step["operation"].string=="messages.send_effect" { names.append("messages.send_effect."+step["variant"]["effect"].string) }
            if step["operation"].string=="messages.send_formatted" {
                names += Set(step["variant"]["formatting"].array.flatMap { $0["styles"].array.map(\.string)+($0["effect"].string.isEmpty ? []:[$0["effect"].string]) }).sorted().map { "messages.send_formatted."+$0 }
            }
        }
        return names
    }
    static func coverage() -> [JSON] {
        operations.map { operation in
            let step=proof()["steps"].array.first { $0["operation"].string==operation }
            return .object(["operation":.string(operation),"proven":.bool(proven(operation)),"reason":.string(step?["reason"].string ?? "per_os_variant_proof_required")])
        } + capabilities().filter { !operations.contains($0) }.map { .object(["operation":.string($0),"proven":.bool(true),"reason":.string("proven_parameter_on_current_os_build")]) }
    }
    static func range(_ entry: JSON, text: String) throws -> CFRange {
        let start=entry["utf16_location"].number,length=entry["utf16_length"].number,units=Array(text.utf16)
        guard start.isFinite,length.isFinite,start.rounded()==start,length.rounded()==length,start>=0,length>0,start+length<=Double(units.count) else { throw SendVariantFailure.invalidVariant }
        let from=Int(start),end=Int(start+length)
        guard !(from>0 && from<units.count && (0xD800...0xDBFF).contains(units[from-1]) && (0xDC00...0xDFFF).contains(units[from])),!(end<units.count && (0xD800...0xDBFF).contains(units[end-1]) && (0xDC00...0xDFFF).contains(units[end])) else { throw SendVariantFailure.invalidVariant }
        return CFRange(location:from,length:Int(length))
    }
    static func validate(_ args: JSON) throws {
        guard let operation=operation(args),args["resend_of"].string.isEmpty,args["text"].string.utf16.count<=512 else { throw SendVariantFailure.invalidVariant }
        let variant=args["variant"],text=args["text"].string
        guard Set(variant.object.keys).isSubset(of:Set(["kind","scheduled_for","effect","formatting","handle_id","utf16_location","utf16_length"])) else { throw SendVariantFailure.invalidVariant }
        if operation=="messages.send_audio" { guard text.isEmpty,args["files"].array.count==1 else { throw SendVariantFailure.invalidVariant } }
        else { guard !text.isEmpty,args["files"].array.isEmpty else { throw SendVariantFailure.invalidVariant } }
        switch variant["kind"].string {
        case "effect": guard effects.contains(variant["effect"].string) else { throw SendVariantFailure.invalidVariant }
        case "formatting":
            guard !variant["formatting"].array.isEmpty,variant["formatting"].array.count<=32 else { throw SendVariantFailure.invalidVariant }
            var actions=0
            for entry in variant["formatting"].array {
                actions+=entry["styles"].array.count+(entry["effect"].string.isEmpty ? 0:1)
                guard actions<=12 else { throw SendVariantFailure.invalidVariant }
                _=try range(entry,text:text)
                let names=entry["styles"].array.map(\.string)
                guard Set(names).count==names.count,Set(names).isSubset(of:Set(styles)),!names.isEmpty || !entry["effect"].string.isEmpty,entry["effect"].string.isEmpty || textEffects.contains(entry["effect"].string) else { throw SendVariantFailure.invalidVariant }
            }
        case "schedule": guard let date=parseTimestamp(variant["scheduled_for"].string),date>Date(),date<=Date().addingTimeInterval(14*86400) else { throw SendVariantFailure.invalidVariant }
        case "mention": _=try range(variant,text:text);guard !variant["handle_id"].string.isEmpty else { throw SendVariantFailure.invalidVariant }
        case "audio": break
        default: throw SendVariantFailure.invalidVariant
        }
    }
    public static func probeAudio(target: String) async throws { try await MessagesVariantUI.send(.object(["variant":.object(["kind":.string("audio")]),"text":.string("")]),target:target) }
    public static func clearProofDraft(_ tag: String) async throws -> Bool {
        guard tag.range(of:#"^\[ab-test [A-Za-z0-9_-]+\]$"#,options:.regularExpression) != nil else { throw SourceFailure.invalid }
        return try await MessagesAccessibility.clearTaggedDraft(tag)
    }
    static func dispatch(_ prepared: Prepared, path: String) async throws -> JSON {
        let args=prepared.action["messages"],reader=try MessagesReader(path:path),route=prepared.nativeState["destination"]
        try validate(args)
        guard let operation=operation(args) else { throw SendVariantFailure.invalidVariant }
        let chat=route["chat"].string
        guard !chat.isEmpty,let target=try reader.query("SELECT m.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE c.guid=? ORDER BY m.ROWID DESC LIMIT 1",[chat]).first?["guid"] else { throw SourceFailure.capabilityUnproven }
        let maximum=try reader.maximum()
        guard let row=try reader.message(target),let item=try reader.messageObservations(row).first else { throw SourceFailure.conflict }
        let targetData=try JSONDecoder().decode(JSON.self,from:JSONSerialization.data(withJSONObject:item.2))
        guard let expiresAt=parseTimestamp(prepared.nativeState["expires_at"].string),expiresAt>Date() else { throw SourceFailure.expired }
        do { try await MessagesVariantUI.send(args,target:target,targetData:targetData,expiresAt:expiresAt) }
        catch let failure as SendVariantFailure {
            return .object(["source":.string("messages"),"observed":.bool(false),"disposition":.string(failure == .sendReceiptUnknown ? "indeterminate":"failed_no_effect"),"reason":.string(failure.rawValue)])
        }
        let deadline=ContinuousClock.now.advanced(by:.seconds(10))
        repeat {
            let candidates=try reader.query("SELECT m.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE m.ROWID>? AND c.guid=? AND m.is_from_me=1 AND (m.text=? OR instr(m.attributedBody,CAST(? AS BLOB))>0)",[String(maximum),chat,args["text"].string,args["text"].string])
            if candidates.count>1 { break }
            if let key=candidates.first?["guid"],let row=try reader.message(key),let captured=try reader.messageObservations(row).first {
                let data=try JSONDecoder().decode(JSON.self,from:JSONSerialization.data(withJSONObject:captured.2))
                if data["error_code"].number != 0 {
                    return .object(["source":.string("messages"),"collection":.string("messages.message"),"native_key":.string(key),"observed":.bool(true),"disposition":.string("delivery_failed"),"reason":.string("native_delivery_failed"),"parts":.array([.object(["index":.number(0),"kind":.string("text"),"native_key":.string(key),"observed":.bool(true),"disposition":.string("delivery_failed")])])])
                }
                var matched=false
                if operation=="messages.send_effect" { matched=data["effect"].string.lowercased().replacingOccurrences(of:"invisibleink",with:"invisible-ink").hasSuffix(args["variant"]["effect"].string) }
                if operation=="messages.send_formatted" {
                    matched=args["variant"]["formatting"].array.allSatisfy { expected in data["formatting"].array.contains { actual in actual["utf16_location"]==expected["utf16_location"] && actual["utf16_length"]==expected["utf16_length"] && Set(expected["styles"].array.map(\.string)).isSubset(of:Set(actual["styles"].array.map(\.string))) && (expected["effect"].string.isEmpty || actual["effect"]==expected["effect"]) } }
                }
                if matched { return .object(["source":.string("messages"),"collection":.string("messages.message"),"native_key":.string(key),"observed":.bool(true),"disposition":.string("locally_observed"),"parts":.array([.object(["index":.number(0),"kind":.string("text"),"native_key":.string(key),"observed":.bool(true),"disposition":.string("locally_observed")])])]) }
            }
            try await Task.sleep(for:.milliseconds(100))
        } while ContinuousClock.now<deadline
        return .object(["source":.string("messages"),"observed":.bool(false),"disposition":.string("indeterminate"),"reason":.string(operation=="messages.send_effect" ? "effect_readback_unverified":"formatting_readback_unverified")])
    }
}
