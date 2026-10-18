import Foundation
import AppKit
import Carbon

public enum MessagesUIFailure: String, Error, Sendable {
    case targetUnavailable, targetAmbiguous, navigationUnavailable, automationDenied, menuUnavailable, editFieldUnavailable, messageValueRejected, composerUnavailable, recipientMismatch, wrongService, draftPresent
}

enum MessagesNavigation {
    static var application: NSRunningApplication? {
        if let front = NSWorkspace.shared.frontmostApplication, front.bundleIdentifier == "com.apple.MobileSMS" { return front }
        return NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.MobileSMS").first
    }
    static func open(_ url: URL) throws {
        guard let application else { throw MessagesUIFailure.navigationUnavailable }
        let event = NSAppleEventDescriptor(eventClass: AEEventClass(kInternetEventClass), eventID: AEEventID(kAEGetURL), targetDescriptor: NSAppleEventDescriptor(processIdentifier: application.processIdentifier), returnID: AEReturnID(kAutoGenerateReturnID), transactionID: AETransactionID(kAnyTransactionID))
        event.setParam(NSAppleEventDescriptor(string: url.absoluteString), forKeyword: AEKeyword(keyDirectObject))
        do { _ = try event.sendEvent(options: [.neverInteract, .noReply], timeout: 3) }
        catch { throw (error as NSError).code == -1743 ? MessagesUIFailure.automationDenied : MessagesUIFailure.navigationUnavailable }
        _ = application.activate()
    }
}
