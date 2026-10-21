import Foundation
import Decoders

struct RichDecoderChecks {
    static func run(_ root: String) throws {
        func load(_ name: String) throws -> Data { try Data(contentsOf: URL(fileURLWithPath: root + "/" + name)) }
        func require(_ condition: Bool, _ label: String) { if !condition { print("rich decoder failed: " + label); exit(1) } }
        require(MessageRich.groupEventKind(item: 1, action: 0, hasMember: true, senderIsMember: false, title: "") == "member_added", "group member mapping")
        require(MessageRich.groupEventKind(item: 1, action: 0, hasMember: true, senderIsMember: true, title: "") == "phone_changed", "self member phone change")
        require(MessageRich.groupEventKind(item: 99, action: 0, hasMember: false, senderIsMember: false, title: "") == "unknown", "unknown group action")
        for (bundle, kind) in [("URLBalloonProvider", "link"), ("MSMessageExtensionBalloonPlugin:synthetic", "app"), ("DigitalTouchBalloonProvider", "digital_touch"), ("Handwriting", "handwriting"), ("Poll", "poll"), ("SafetyMonitor", "check_in"), ("PeerPayment", "apple_cash"), ("FindMy", "find_my"), ("chatbot", "chatbot"), ("synthetic.unknown", "unknown")] { require(MessageRich.balloonKind(bundle) == kind, "balloon kind " + kind) }
        for (item, action, kind) in [(1, 1, "member_removed"), (3, 0, "member_left"), (3, 1, "photo_changed"), (3, 2, "photo_removed")] { require(MessageRich.groupEventKind(item: item, action: action, hasMember: true, senderIsMember: false, title: "") == kind, "group event " + kind) }
        let bytes = try load("rich-link.plist")
        let balloon = MessageRich.balloon(bundle: "com.apple.messages.URLBalloonProvider", payload: bytes, attachmentGUIDs: ["synthetic-image", "synthetic-icon"])
        let link = balloon?["link"] as? [String: Any]
        require(link?["url"] as? String == "https://example.invalid/preview" && link?["title"] as? String == "Synthetic link" && link?["image_attachment_id"] as? String == "synthetic-image" && link?["icon_attachment_id"] as? String == "synthetic-icon", "link UID graph and payload images")
        require(link?["summary"] as? String == "Synthetic summary" && link?["site_name"] as? String == "Example" && link?["video_url"] as? String == "https://example.invalid/video", "link summary site and video")
        for length in 0..<bytes.count { require(MessageArchive.decode(bytes.prefix(length)) == nil, "truncated keyed archive") }
        let location = MessageRich.balloon(bundle: "com.apple.messages.URLBalloonProvider", payload: try load("rich-location.plist"))
        require(location?["kind"] as? String == "find_my" && location?["link"] != nil, "map specialization")
        let app = MessageRich.balloon(bundle: "com.apple.messages.MSMessageExtensionBalloonPlugin:synthetic:example.app", payload: try load("rich-app.plist"), attachmentGUIDs: ["synthetic-image"])
        require((app?["app"] as? [String: Any])?["caption"] as? String == "Synthetic app", "template captions")
        let appFields = app?["app"] as? [String: Any]
        require(appFields?["subcaption"] as? String == "Synthetic detail" && appFields?["trailing_caption"] as? String == "Synthetic trailing" && appFields?["url"] as? String == "https://example.invalid/app" && appFields?["image_attachment_id"] as? String == "synthetic-image", "app template optional fields")
        let sticker = MessageRich.sticker(try load("rich-sticker.plist"), target: "p:2/synthetic-parent")
        require(sticker?["target_message_id"] as? String == "synthetic-parent" && sticker?["x"] as? Double == 0.25 && sticker?["part_index"] as? Int == 2, "sticker transform")
        require(MessageRich.sticker(Data([0,1]), target: "synthetic") == nil, "malformed sticker")
        for name in ["cycle", "bad-reference"] { require(MessageArchive.decode(try load("rich-" + name + ".plist")) == nil, "invalid archive graph " + name) }
        let properties = MessageRich.chatProperties(try load("rich-chat.plist"))
        require(properties["read_receipts"] as? String == "on" && properties["group_photo_attachment_id"] as? String == "synthetic-photo" && properties["pinned_index"] is NSNull, "chat properties and unavailable state")
        let airLink = MessageRich.balloon(bundle: "URLBalloonProvider", payload: try load("rich-air-link.plist"), attachmentGUIDs: ["synthetic-image", "synthetic-icon"])
        require((airLink?["link"] as? [String: Any])?["title"] as? String == "Synthetic preview", "scrubbed Air link caption")
        let airApp = MessageRich.balloon(bundle: "MSMessageExtensionBalloonPlugin:synthetic", payload: try load("rich-air-app.plist"), attachmentGUIDs: ["synthetic-image"])
        require((airApp?["app"] as? [String: Any])?["caption"] as? String == "Synthetic preview", "scrubbed Air app caption")
        for label in ["formatting", "effect", "genmoji"] {
            let data = try load("rich-air-" + label + ".typedstream")
            let body = TypedStream.decode(data)
            require(try FixtureScrubber.scrub(data) == data, "scrubbed Air typedstream")
            require(body.decodeStatus == "decoded", "Air rich typedstream decoded")
            if label == "formatting" { require(body.formatting.contains(where: { !$0.styles.isEmpty }), "Air formatting") }
            if label == "effect" { require(body.formatting.contains(where: { $0.effect != nil }), "Air text effect") }
            if label == "genmoji" { require(body.parts.contains(where: { $0.kind == "genmoji" }), "Air Genmoji") }
        }
        let body = TypedStream.decode(try load("rich-formatting.typedstream"))
        require(body.decodeStatus == "decoded" && body.formatting.contains(where: { $0.styles == ["bold", "italic"] }) && body.formatting.contains(where: { $0.utf16Location == 2 && $0.styles.contains("underline") }), "overlapping UTF16 ranges")
        require(Set(body.formatting.compactMap(\.effect)) == Set(["big", "small", "shake", "nod", "explode", "ripple", "bloom", "jitter"]), "all text effect IDs")
        let genmoji = TypedStream.decode(try load("rich-genmoji.typedstream"))
        require(genmoji.parts.contains(where: { $0.kind == "genmoji" }), "genmoji glyph")
    }
}
