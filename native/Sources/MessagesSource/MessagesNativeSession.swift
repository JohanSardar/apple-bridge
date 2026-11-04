import Foundation
import ObjectiveC
import Darwin
import SourceSupport
import Decoders

@MainActor
enum MessagesNativeSession {
    private static let forwardedImplementation: IMP? = {
        guard let runtime = dlopen("/usr/lib/libobjc.A.dylib", RTLD_LAZY), let implementation = dlsym(runtime, "objc_msgSend") else { return nil }
        return unsafeBitCast(implementation, to: IMP.self)
    }()
    private static func method(_ object: AnyObject, _ name: String) throws -> (Selector, IMP) {
        let selector = NSSelectorFromString(name)
        let type: AnyClass? = object_getClass(object)
        if let type, let method = class_getInstanceMethod(type, selector) { return (selector, method_getImplementation(method)) }
        if (object as? NSObjectProtocol)?.responds(to: selector) == true, let implementation = forwardedImplementation { return (selector, implementation) }
        let receiver = type.map { BodyDigest.hash(Data(NSStringFromClass($0).utf8)) } ?? ""
        try? PrivateReport.write(["selector_digest": BodyDigest.hash(Data(name.utf8)), "receiver_digest": receiver, "selector_available": false], to: PrivateReport.directory.appendingPathComponent("messages-native-method-diagnostic.json"))
        throw SourceFailure.unsupported
    }
    private static func object(_ receiver: AnyObject, _ name: String, argument: AnyObject? = nil) throws -> AnyObject? {
        let (selector, implementation) = try method(receiver, name)
        if name.contains(":") {
            typealias Call = @convention(c) (AnyObject, Selector, AnyObject?) -> Unmanaged<AnyObject>?
            return unsafeBitCast(implementation, to: Call.self)(receiver, selector, argument)?.takeUnretainedValue()
        }
        typealias Call = @convention(c) (AnyObject, Selector) -> Unmanaged<AnyObject>?
        return unsafeBitCast(implementation, to: Call.self)(receiver, selector)?.takeUnretainedValue()
    }
    private static func flag(_ receiver: AnyObject, _ name: String) throws -> Bool {
        let (selector, implementation) = try method(receiver, name)
        typealias Call = @convention(c) (AnyObject, Selector) -> Bool
        return unsafeBitCast(implementation, to: Call.self)(receiver, selector)
    }
    private static func checkpoint(_ stage: String) {
        try? PrivateReport.write(["stage": stage], to: PrivateReport.directory.appendingPathComponent("messages-native-session-diagnostic.json"))
    }
    static func deleteOwnedItem(_ guid: String, chat chatGUID: String) throws {
        guard UUID(uuidString: guid) != nil, !chatGUID.isEmpty,
              dlopen("/System/Library/PrivateFrameworks/IMCore.framework/IMCore", RTLD_LAZY) != nil,
              let type = NSClassFromString("IMDaemonController"), let controller = try object(type, "sharedController") else { throw SourceFailure.invalid }
        _ = try flag(controller, "connectToDaemon")
        let deadline = Date().addingTimeInterval(5)
        while (try? flag(controller, "isConnected")) != true, Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.05)) }
        guard try flag(controller, "isConnected"), let daemon = try object(controller, "synchronousRemoteDaemon") else {
            checkpoint("owned_cleanup_connection_unavailable")
            throw SourceFailure.unavailable
        }
        let (selector, implementation) = try method(daemon, "moveMessagesWithGUIDsToRecentlyDeleted:deleteDate:queryID:")
        typealias Move = @convention(c) (AnyObject, Selector, NSArray, NSDate, NSString) -> Void
        checkpoint("owned_item_soft_delete_requested")
        unsafeBitCast(implementation, to: Move.self)(daemon, selector, [guid] as NSArray, NSDate(), UUID().uuidString as NSString)
    }
}
