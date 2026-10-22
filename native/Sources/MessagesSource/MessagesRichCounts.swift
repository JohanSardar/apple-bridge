#if LAB
import Foundation
import Decoders

extension MessagesProbes {
    static func richCounts() throws -> [String: Any] {
        let reader = try MessagesReader(path: NSHomeDirectory() + "/Library/Messages/chat.db")
        var counts: [String: Int] = [:]
        let fields = ["is_audio_message", "schedule_type", "subject", "was_delivered_quietly", "did_notify_recipient", "is_spam", "has_unseen_mention", "is_stewie", "sent_or_received_off_grid", "is_sos"]
        let whereFlags = fields.filter { reader.columns["message"]?.contains($0) == true }.map { "COALESCE(\($0),0) NOT IN (0,'')" }
        let rows = try reader.query("SELECT guid FROM message WHERE COALESCE(balloon_bundle_id,'')!='' OR instr(attributedBody,'__kIMText')>0 OR instr(attributedBody,'__kIMEmojiImage')>0 OR EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.message_id=message.ROWID AND r.delete_date>0) OR " + whereFlags.joined(separator: " OR "))
        for key in ["link", "app", "digital_touch", "handwriting", "poll", "check_in", "apple_cash", "find_my", "chatbot", "unknown"] { counts["balloon." + key] = 0 }
        for key in ["balloon.payload_decoded", "balloon.link_card", "balloon.app_caption", "balloon.image_reference", "audio", "audio.played", "audio.expirable", "schedule", "subject", "delivery.quietly", "delivery.notified_anyway", "is_spam", "has_unseen_mention", "via_satellite", "is_sos", "formatting", "genmoji", "attachment.is_sticker", "attachment.hidden", "attachment.genmoji", "attachment.sticker", "chat.unknown", "chat.is_spam", "chat.group_photo", "chat.read_receipts", "message.deleted_at", "chat.deleted_at"] { counts[key] = 0 }
        for key in ["title", "summary", "site_name", "video_url", "icon_attachment_id"] { counts["balloon.link." + key] = 0 }
        for key in ["subcaption", "trailing_caption", "url"] { counts["balloon.app." + key] = 0 }
        for row in rows {
            guard let row = try reader.message(row["guid"]) else { continue }
            for (collection, _, data) in try reader.messageObservations(row) where collection == "messages.message" {
                if let balloon = data["balloon"] as? [String: Any], let kind = balloon["kind"] as? String {
                    counts["balloon." + kind, default: 0] += 1
                    if balloon["decode_status"] as? String == "decoded" { counts["balloon.payload_decoded", default: 0] += 1 }
                    if let link = balloon["link"] as? [String: Any] {
                        counts["balloon.link_card", default: 0] += 1
                        for key in ["title", "summary", "site_name", "video_url", "icon_attachment_id"] where link[key] != nil { counts["balloon.link." + key, default: 0] += 1 }
                    }
                    if let app = balloon["app"] as? [String: Any] { for key in ["subcaption", "trailing_caption", "url"] where app[key] != nil { counts["balloon.app." + key, default: 0] += 1 } }
                    if (balloon["app"] as? [String: Any])?["caption"] != nil { counts["balloon.app_caption", default: 0] += 1 }
                    if ((balloon["app"] as? [String: Any])?["image_attachment_id"] ?? (balloon["link"] as? [String: Any])?["image_attachment_id"]) != nil { counts["balloon.image_reference", default: 0] += 1 }
                }
                if let audio = data["audio"] as? [String: Any], audio["is_audio_message"] as? Bool == true { counts["audio", default: 0] += 1
                    if audio["played_at"] != nil { counts["audio.played", default: 0] += 1 }
                    if audio["expirable"] as? Bool == true { counts["audio.expirable", default: 0] += 1 }
                }
                for key in ["schedule", "subject"] { if let v = data[key], !(v is NSNull), (v as? String) != "" { counts[key, default: 0] += 1 } }
                for key in ["is_spam", "has_unseen_mention", "via_satellite", "is_sos"] where data[key] as? Bool == true { counts[key, default: 0] += 1 }
                for key in ["quietly", "notified_anyway"] where (data["delivery"] as? [String: Bool])?[key] == true { counts["delivery." + key, default: 0] += 1 }
                if data["deleted_at"] != nil { counts["message.deleted_at", default: 0] += 1 }
                if let ranges = data["formatting"] as? [[String: Any]], !ranges.isEmpty { counts["formatting", default: 0] += 1 }
                if let parts = data["parts"] as? [[String: Any]], parts.contains(where: { $0["kind"] as? String == "genmoji" }) { counts["genmoji", default: 0] += 1 }
            }
        }
        for (collection, _, data) in try reader.smallObservations() {
            if collection == "messages.attachment" {
                for key in ["is_sticker", "hidden", "genmoji"] where data[key] as? Bool == true { counts["attachment." + key, default: 0] += 1 }
                if data["sticker"] != nil { counts["attachment.sticker", default: 0] += 1 }
            }
            if collection == "messages.chat" {
                if data["filter"] as? String == "unknown" { counts["chat.unknown", default: 0] += 1 }
                if data["is_spam"] as? Bool == true { counts["chat.is_spam", default: 0] += 1 }
                if data["group_photo_attachment_id"] != nil { counts["chat.group_photo", default: 0] += 1 }
                if data["read_receipts"] as? String != "default" { counts["chat.read_receipts", default: 0] += 1 }
                if data["deleted_at"] != nil { counts["chat.deleted_at", default: 0] += 1 }
            }
        }
        return ["status": "verified", "counts": counts]
    }
}
#endif
