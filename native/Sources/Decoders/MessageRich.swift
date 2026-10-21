import Foundation

public enum MessageRich {
    public static func groupEventKind(item: Int, action: Int, hasMember: Bool, senderIsMember: Bool, title: String) -> String {
        if item == 2 && !title.isEmpty { return "rename" }
        if item == 1 && hasMember {
            if action == 1 { return "member_removed" }
            if action == 0 { return senderIsMember ? "phone_changed" : "member_added" }
        }
        if item == 3 { return [0: "member_left", 1: "photo_changed", 2: "photo_removed"][action] ?? "unknown" }
        return "unknown"
    }
    public static func balloonKind(_ bundle: String) -> String {
        let value = bundle.lowercased()
        if value.contains("urlballoon") { return "link" }
        if value.contains("digitaltouch") { return "digital_touch" }
        if value.contains("handwriting") { return "handwriting" }
        if value.contains("peerpayment") || value.contains("applecash") { return "apple_cash" }
        if value.contains("findmy") { return "find_my" }
        if value.contains("checkin") || value.contains("safetymonitor") { return "check_in" }
        if value.contains("poll") { return "poll" }
        if value.contains("chatbot") { return "chatbot" }
        if value.contains("msmessageextension") { return "app" }
        return "unknown"
    }
    public static func balloon(bundle: String, payload: Data?, attachmentGUIDs: [String] = []) -> [String: Any]? {
        guard !bundle.isEmpty else { return nil }
        var value: [String: Any] = ["bundle_id": bundle, "kind": balloonKind(bundle)]
        let root = payload.flatMap(MessageArchive.decode) ?? [:]
        if let name = root["an"] as? String { value["app_name"] = name }
        else if bundle.contains("gamepigeon") { value["app_name"] = "GamePigeon" }
        else if bundle.contains("youtube") { value["app_name"] = "YouTube" }
        else if bundle.contains("PhotosMessagesApp") { value["app_name"] = "Photos" }
        value["decode_status"] = payload == nil ? "unavailable" : root.isEmpty ? "undecoded" : "decoded"
        if let payload, root.isEmpty { value["raw_payload_digest"] = BodyDigest.hash(payload) }
        let metadata = root["richLinkMetadata"] as? [String: Any] ?? root
        if balloonKind(bundle) == "link" {
            if let specialization = metadata["specialization"] as? [String: Any], specialization["address"] != nil || specialization["addressComponents"] != nil || specialization["isPointOfInterest"] != nil { value["kind"] = "find_my" }
            var link: [String: Any] = [:]
            for (key, output) in [("URL", "url"), ("title", "title"), ("summary", "summary"), ("siteName", "site_name")] { link[output] = metadata[key] as? String }
            if link["url"] == nil { link["url"] = metadata["originalURL"] as? String }
            let video = metadata["video"] as? [String: Any] ?? metadata["videoMetadata"] as? [String: Any] ?? [:]
            link["video_url"] = video["URL"] as? String ?? metadata["youTubeURL"] as? String
            for (key, field) in [("image", "image_attachment_id"), ("icon", "icon_attachment_id")] {
                if let image = metadata[key] as? [String: Any], let index = image["richLinkImageAttachmentSubstituteIndex"] as? Int, attachmentGUIDs.indices.contains(index) { link[field] = attachmentGUIDs[index] }
            }
            if link["url"] != nil { value["link"] = link }
        } else {
            let layout = root["layout"] as? [String: Any] ?? root
            var app: [String: Any] = [:]
            for (key, output) in [("caption", "caption"), ("subcaption", "subcaption"), ("secondary-subcaption", "trailing_caption"), ("URL", "url")] { app[output] = layout[key] as? String }
            if app["caption"] == nil { app["caption"] = layout["ldtext"] as? String }
            if let image = attachmentGUIDs.first { app["image_attachment_id"] = image }
            if !app.isEmpty { value["app"] = app }
        }
        return value
    }
    public static func sticker(_ data: Data, target: String) -> [String: Any]? {
        guard let value = MessageArchive.decode(data), !target.isEmpty else { return nil }
        func number(_ key: String) -> Double? {
            let n = (value[key] as? NSNumber)?.doubleValue ?? (value[key] as? String).flatMap(Double.init)
            guard let n, n.isFinite else { return nil }
            return n
        }
        guard let x = number("sxs"), let y = number("sys"), let scale = number("ssa"), let rotation = number("sro"), scale > 0 else { return nil }
        var guid = target
        var part = number("sai").flatMap { Int(exactly: $0) } ?? 0
        if target.hasPrefix("p:"), let slash = target.firstIndex(of: "/") {
            guard let index = Int(target[target.index(target.startIndex, offsetBy: 2)..<slash]), index >= 0 else { return nil }
            part = index
            guid = String(target[target.index(after: slash)...])
        } else if target.hasPrefix("bp:") { guid = String(target.dropFirst(3)) }
        guard !guid.isEmpty, part >= 0 else { return nil }
        return ["target_message_id": guid, "part_index": part, "x": x, "y": y, "scale": scale, "rotation": rotation]
    }
    public static func chatProperties(_ data: Data?) -> [String: Any] {
        let properties = data.flatMap(MessageArchive.decode) ?? [:]
        var result: [String: Any] = ["read_receipts": "default", "muted_until": NSNull(), "pinned_index": NSNull()]
        if let flag = properties["EnableReadReceiptForChat"] as? NSNumber { result["read_receipts"] = flag.boolValue ? "on" : "off" }
        if let photo = properties["groupPhotoGuid"] as? String, !photo.isEmpty { result["group_photo_attachment_id"] = photo }
        return result
    }
}
