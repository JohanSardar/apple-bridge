import Foundation
import MessagesSource
import SourceSupport

@main
struct MessagesSafetyChecks {
    static func main() async throws {
        var checks = try await AccessibilityInputChecks.run()
        checks["multipart_capture_repeats_all_parts_in_one_batch"] = try await CommandCaptureChecks.run()
        checks["self_echo_does_not_cancel_same_media_render"] = !Messages.shouldPreemptMedia(activeDigest: "same", incomingDigest: "same", activeLive: true, incomingImage: true)
        checks["same_media_backfill_is_reused"] = !Messages.shouldPreemptMedia(activeDigest: "same", incomingDigest: "same", activeLive: false, incomingImage: true)
        checks["new_live_image_still_preempts_other_media"] = Messages.shouldPreemptMedia(activeDigest: "old", incomingDigest: "new", activeLive: true, incomingImage: true)
        checks["new_live_media_still_preempts_backfill"] = Messages.shouldPreemptMedia(activeDigest: "old", incomingDigest: "new", activeLive: false, incomingImage: false)
        print(String(decoding: try JSONEncoder().encode(checks), as: UTF8.self))
    }
}
