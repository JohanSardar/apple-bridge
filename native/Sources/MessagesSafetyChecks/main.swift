import Foundation
import MessagesSource
import SourceSupport

@main
struct MessagesSafetyChecks {
    static func main() async throws {
        let checks = try await AccessibilityInputChecks.run()
        print(String(decoding: try JSONEncoder().encode(checks), as: UTF8.self))
    }
}
