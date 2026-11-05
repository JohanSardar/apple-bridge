import Foundation
import AppKit
import Carbon

private final class PermissionReply: @unchecked Sendable {
    private let lock = NSLock()
    private var continuation: CheckedContinuation<Int32, Never>?
    init(_ continuation: CheckedContinuation<Int32, Never>) { self.continuation = continuation }
    var pending: Bool { lock.withLock { continuation != nil } }
    func finish(_ status: Int32) {
        let reply = lock.withLock {
            let reply = continuation
            continuation = nil
            return reply
        }
        reply?.resume(returning: status)
    }
}

public enum AppleScriptRunner {
    private static let permissionQueue = DispatchQueue(label: "apple-bridge.automation-permission")
    private static let timedOut: Int32 = -1712

    public static func automationStatus(after interval: TimeInterval = 10, check: @escaping @Sendable () -> Int32) async -> Int32 {
        await withCheckedContinuation { continuation in
            let reply = PermissionReply(continuation)
            permissionQueue.async {
                guard reply.pending else { return }
                reply.finish(check())
            }
            DispatchQueue.global().asyncAfter(deadline: .now() + interval) { reply.finish(timedOut) }
        }
    }

    @MainActor
    private static func launch(_ bundle: String) async -> Bool {
        guard NSRunningApplication.runningApplications(withBundleIdentifier: bundle).isEmpty else { return true }
        guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundle) else { return false }
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.activates = false
        return (try? await NSWorkspace.shared.openApplication(at: url, configuration: configuration)) != nil
    }

    @MainActor
    public static func run(_ source: String, target bundle: String? = nil) async throws -> JSON {
        try Task.checkCancellation()
        func failed(_ number: Int) -> JSON { .object(["error_number": .number(Double(number))]) }
        if let bundle {
            guard await launch(bundle) else { return failed(-600) }
            let status = await automationStatus {
                let target = NSAppleEventDescriptor(bundleIdentifier: bundle)
                return AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), false)
            }
            guard status == noErr else { return failed(Int(status)) }
        }
        var error: NSDictionary?
        guard let script = NSAppleScript(source: source) else { throw SourceFailure.invalid }
        let result = script.executeAndReturnError(&error)
        if let error { return failed(error["NSAppleScriptErrorNumber"] as? Int ?? 0) }
        return .object(["result": .string(result.stringValue ?? "")])
    }
}
