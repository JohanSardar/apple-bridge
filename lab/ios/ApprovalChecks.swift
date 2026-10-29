import Foundation

@MainActor
final class FakeAppleApproval: AppleApprovalAdapter {
    var calls = 0
    var authorized = true
    func perform(_ command: String, raw: Data) async throws -> Data {
        guard authorized else { throw AppleApprovalError.needsAuthorization }
        calls += 1
        return Data("{\"count\":1}".utf8)
    }
}

@main
struct ApprovalChecks {
    @MainActor
    static func main() async throws {
        let plist = try Data(contentsOf: URL(fileURLWithPath: "ios/App/Info.plist"))
        let config = try PropertyListSerialization.propertyList(from: plist, format: nil) as! [String: Any]
        for feature in ApprovalFeature.allCases {
            precondition(config["ABApproved_\(feature.rawValue)"] as? Bool == false)
        }
        let adapter = FakeAppleApproval()
        let commands = ["iphone.wallet.account", "iphone.wallet.transaction", "iphone.critical.alert", "iphone.location.push", "iphone.fall.detection"]
        for command in commands {
            for enabled in [false, true] {
                do {
                    _ = try await AppleApprovalService.execute(command, raw: Data(), enabled: enabled, approved: false, adapter: adapter)
                    fatalError("unapproved accepted")
                } catch AppleApprovalError.needsAppleApproval {}
            }
            do {
                _ = try await AppleApprovalService.execute(command, raw: Data(), enabled: false, approved: true, adapter: adapter)
                fatalError("disabled accepted")
            } catch AppleApprovalError.disabled {}
            precondition(adapter.calls == 0)
        }
        for command in commands {
            let data = try await AppleApprovalService.execute(command, raw: Data(), enabled: true, approved: true, adapter: adapter)
            precondition(data == Data("{\"count\":1}".utf8))
        }
        precondition(adapter.calls == commands.count)
        adapter.authorized = false
        do {
            _ = try await AppleApprovalService.execute("iphone.wallet.account", raw: Data(), enabled: true, approved: true, adapter: adapter)
            fatalError("unauthorized accepted")
        } catch AppleApprovalError.needsAuthorization {}
        precondition(adapter.calls == commands.count)
        print("PASS")
    }
}
