import Foundation
import Decoders

@main
struct DecoderChecks {
    static func main() throws {
        let root = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "lab/fixtures/messages"
        let data = try Data(contentsOf: URL(fileURLWithPath: root + "/synthetic.typedstream"))
        let body = TypedStream.decode(data)
        guard body.decodeStatus == "decoded", body.text == "[ab-test decoder] Hello 🧪 שלום e\u{301} tail", body.parts.contains(where: { $0.attachmentGUID == "synthetic-attachment" }), body.parts.contains(where: { $0.mention == "self@example.com" }) else {
            print("decoder checks failed: attributed ranges")
            exit(1)
        }
        for length in 0..<data.count {
            let truncated = TypedStream.decode(data.prefix(length))
            guard truncated.decodeStatus == "undecoded", truncated.text == nil, truncated.rawBodyDigest != nil else {
                print("decoder checks failed: truncated body")
                exit(1)
            }
        }
        let malformed = TypedStream.decode(try Data(contentsOf: URL(fileURLWithPath: root + "/malformed.typedstream")))
        guard malformed.decodeStatus == "undecoded", malformed.text == nil, malformed.rawBodyDigest != nil else { print("decoder checks failed: malformed fixture"); exit(1) }
        let long = TypedStream.decode(try Data(contentsOf: URL(fileURLWithPath: root + "/long.typedstream")))
        guard long.decodeStatus == "decoded", long.text?.utf8.count ?? 0 > 100_000 else { print("decoder checks failed: long Unicode"); exit(1) }
        let scrubbed = try FixtureScrubber.scrub(data)
        let scrubbedBody = TypedStream.decode(scrubbed)
        guard scrubbedBody.decodeStatus == "decoded", scrubbedBody.text?.utf16.count == body.text?.utf16.count, scrubbedBody.text != body.text else { print("decoder checks failed: scrubbing"); exit(1) }
        for index in 0...7 {
            for removed in [false, true] {
                let reaction = MessageReaction.decode(type: (removed ? 3000 : 2000) + index, target: "p:2/synthetic-message", emoji: index == 6 ? "🧪" : nil)
                guard reaction?.partIndex == 2, reaction?.removed == removed, reaction?.messageGUID == "synthetic-message" else { print("decoder checks failed: reaction"); exit(1) }
            }
        }
        guard MessageReaction.decode(type: 1999, target: "p:0/a", emoji: nil) == nil,
              MessageReaction.decode(type: 2000, target: "p:bad/a", emoji: nil) == nil else { print("decoder checks failed: invalid reaction"); exit(1) }
        let summary = MessageSummary.decode(try Data(contentsOf: URL(fileURLWithPath: root + "/edited-unsent.plist")))
        guard summary.decodeStatus == "decoded", summary.editHistory.count == 1, summary.retractedParts == [1, 3], summary.editHistory.first?.body.text == body.text else { print("decoder checks failed: edit history"); exit(1) }
        guard MessageSummary.decode(Data([0, 1, 2])).decodeStatus == "undecoded" else { print("decoder checks failed: malformed summary"); exit(1) }
        let malformedHistory = MessageSummary.decode(try Data(contentsOf: URL(fileURLWithPath: root + "/malformed-edit-history.plist")))
        guard malformedHistory.decodeStatus == "undecoded", malformedHistory.rawBodyDigest != nil, malformedHistory.editHistory.isEmpty else { print("decoder checks failed: wrong typed edit history"); exit(1) }
        var mentions = 0
        var attachments = 0
        for file in try FileManager.default.contentsOfDirectory(atPath: root) where file.hasPrefix("air-") && file.hasSuffix(".typedstream") {
            let captured = try Data(contentsOf: URL(fileURLWithPath: root + "/" + file))
            let decoded = TypedStream.decode(captured)
            guard try FixtureScrubber.scrub(captured) == captured, decoded.decodeStatus == "decoded" else { print("decoder checks failed: captured fixture"); exit(1) }
            if file.hasPrefix("air-mention-") { guard decoded.parts.contains(where: { $0.mention != nil }) else { print("decoder checks failed: captured mention"); exit(1) }; mentions += 1 }
            if file.hasPrefix("air-attachment-") { guard decoded.parts.contains(where: { $0.attachmentGUID != nil }) else { print("decoder checks failed: captured attachment"); exit(1) }; attachments += 1 }
        }

        for file in try FileManager.default.contentsOfDirectory(atPath: root) where file.hasPrefix("air-edited-") && file.hasSuffix(".plist") {
            guard MessageSummary.decode(try Data(contentsOf: URL(fileURLWithPath: root + "/" + file))).decodeStatus == "decoded" else { print("decoder checks failed: captured edits"); exit(1) }
        }

        guard mentions > 0, attachments > 0 else { print("decoder checks failed: missing captured coverage"); exit(1) }
        print("all decoder checks passed")
    }
}
