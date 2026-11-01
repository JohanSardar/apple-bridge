import Foundation
import AppKit
import Carbon

public enum MessagesUIFailure: String, Error, Sendable {
    case targetUnavailable, targetAmbiguous, navigationUnavailable, automationDenied, menuUnavailable, editFieldUnavailable, messageValueRejected, composerUnavailable, recipientMismatch, wrongService, draftPresent
}

public enum MessagesNavigation {
    public static func messageURL(_ guid: String) throws -> URL {
        guard !guid.isEmpty else { throw MessagesUIFailure.navigationUnavailable }
        var components = URLComponents()
        components.scheme = "imessage"
        components.path = "open"
        components.queryItems = [URLQueryItem(name: "message-guid", value: guid)]
        guard let url = components.url else { throw MessagesUIFailure.navigationUnavailable }
        return url
    }
    static var application: NSRunningApplication? {
        if let front = NSWorkspace.shared.frontmostApplication, front.bundleIdentifier == "com.apple.MobileSMS" { return front }
        return NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.MobileSMS").first
    }
    static func open(_ url: URL) throws {
        guard let handler = NSWorkspace.shared.urlForApplication(toOpen: url), Bundle(url: handler)?.bundleIdentifier == "com.apple.MobileSMS",
              NSWorkspace.shared.open(url) else { throw MessagesUIFailure.navigationUnavailable }
        let deadline = Date().addingTimeInterval(3)
        while application == nil, Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.05)) }
        _ = application?.activate()
    }
}
