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
        throw SendVariantFailure.audioMessageFlagUnverified
    }
    static func syntheticForwardedFlag(_ receiver: NSObject, selector: String) -> Bool { (try? flag(receiver, selector)) == true }
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
    private static func message(_ transfer: String) throws -> AnyObject {
        guard dlopen("/System/Library/PrivateFrameworks/IMCore.framework/IMCore", RTLD_LAZY) != nil,
              let type = NSClassFromString("IMMessage") else { throw SendVariantFailure.audioMessageFlagUnverified }
        let body = NSMutableAttributedString(string: "\u{FFFC}")
        body.addAttributes([NSAttributedString.Key("__kIMFileTransferGUIDAttributeName"): transfer, NSAttributedString.Key("__kIMMessagePartAttributeName"): 0], range: NSRange(location: 0, length: 1))
        let receiver: AnyObject = type
        let (selector, implementation) = try method(receiver, "instantMessageWithText:messageSubject:fileTransferGUIDs:flags:threadIdentifier:")
        typealias Call = @convention(c) (AnyObject, Selector, NSAttributedString, NSString?, NSArray, UInt64, NSString?) -> Unmanaged<AnyObject>?
        guard let message = unsafeBitCast(implementation, to: Call.self)(receiver, selector, body, nil, [transfer] as NSArray, 0x100005 | (1 << 21), nil)?.takeUnretainedValue(), try flag(message, "isAudioMessage"), try flag(message, "isFromMe") else { throw SendVariantFailure.audioMessageFlagUnverified }
        return message
    }
    static func syntheticFlag() -> Bool { (try? message("synthetic-transfer")) != nil }
    private static func checkpoint(_ stage: String) {
        try? PrivateReport.write(["stage": stage], to: PrivateReport.directory.appendingPathComponent("messages-native-session-diagnostic.json"))
    }
    private static func chat(_ guid: String) throws -> AnyObject {
        checkpoint("framework_requested")
        guard !guid.isEmpty, dlopen("/System/Library/PrivateFrameworks/IMCore.framework/IMCore", RTLD_LAZY) != nil,
              let controllerType = NSClassFromString("IMDaemonController"), let registryType = NSClassFromString("IMChatRegistry"),
              let controller = try object(controllerType, "sharedController"), let registry = try object(registryType, "sharedInstance") else { throw SendVariantFailure.audioMessageFlagUnverified }
        checkpoint("connection_requested")
        _ = try flag(controller, "connectToDaemon")
        var connected = (try? flag(controller, "isConnected")) == true
        let connectionDeadline = Date().addingTimeInterval(5)
        while !connected, Date() < connectionDeadline {
            RunLoop.current.run(until: Date().addingTimeInterval(0.05))
            connected = (try? flag(controller, "isConnected")) == true
        }
        guard connected else {
            checkpoint("connection_unavailable")
            throw SendVariantFailure.audioMessageFlagUnverified
        }
        checkpoint("connection_established")
        let candidates = MessagesSelfTestIsolation.nativeChatCandidates(guid)
        let components = guid.split(separator: ";", maxSplits: 2, omittingEmptySubsequences: false).map(String.init)
        let identifier = candidates.count == 2 ? components[2] : ""
        let deadline = Date().addingTimeInterval(5)
        var chat: AnyObject?
        if !identifier.isEmpty { chat = try object(registry, "loadChatFromDaemonWithChatIdentifier:", argument: identifier as NSString) }
        repeat {
            for candidate in candidates {
                chat = try object(registry, "existingChatWithGUID:", argument: candidate as NSString)
                if chat != nil { break }
            }
            if chat == nil, !identifier.isEmpty { chat = try object(registry, "existingChatWithChatIdentifier:", argument: identifier as NSString) }
            if chat != nil { break }
            RunLoop.current.run(until: Date().addingTimeInterval(0.05))
        } while Date() < deadline
        guard let chat, let actual = try object(chat, "guid") as? String, candidates.contains(where: { $0.caseInsensitiveCompare(actual) == .orderedSame }) else {
            let known = try? object(registry, "allExistingChats")
            let count = (known as? NSArray)?.count ?? (known as? NSSet)?.count ?? -1
            try? PrivateReport.write(["registry_chat_count": count, "route_candidate_count": candidates.count, "chat_resolved": false], to: PrivateReport.directory.appendingPathComponent("messages-native-chat-diagnostic.json"))
            throw SourceFailure.conflict
        }
        checkpoint("self_chat_resolved")
        return chat
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
    static func failedSelfFixture(_ text: String, chat guid: String, register: (String) throws -> Void) throws -> String {
        guard text.range(of: #"^\[ab-test run_[A-Z0-9]{26}\] failed fixture$"#, options: .regularExpression) != nil else { throw SourceFailure.invalid }
        let chat = try chat(guid)
        guard let type = NSClassFromString("IMMessage") else { throw SourceFailure.unavailable }
        let (factorySelector, factoryImplementation) = try method(type, "instantMessageWithText:flags:threadIdentifier:")
        typealias Factory = @convention(c) (AnyObject, Selector, NSAttributedString, UInt64, NSString?) -> Unmanaged<AnyObject>?
        guard let message = unsafeBitCast(factoryImplementation, to: Factory.self)(type, factorySelector, NSAttributedString(string: text), 0x100005, nil)?.takeUnretainedValue(),
              try flag(message, "isFromMe"), let native = try object(message, "guid") as? String, !native.isEmpty else { throw SourceFailure.conflict }
        let (sendSelector, sendImplementation) = try method(chat, "sendMessage:")
        let (cancelSelector, cancelImplementation) = try method(chat, "cancelMessage:")
        try register(native)
        typealias Call = @convention(c) (AnyObject, Selector, AnyObject) -> Void
        unsafeBitCast(sendImplementation, to: Call.self)(chat, sendSelector, message)
        unsafeBitCast(cancelImplementation, to: Call.self)(chat, cancelSelector, message)
        return native
    }
    static func send(_ part: JSON, chat guid: String) throws -> String {
        let chat = try chat(guid)
        guard let transferType = NSClassFromString("IMFileTransferCenter"), let transfers = try object(transferType, "sharedInstance"),
              let transfer = try object(transfers, "guidForNewOutgoingTransferWithLocalURL:", argument: NSURL(fileURLWithPath: part["path"].string)) as? String, !transfer.isEmpty else { throw SendVariantFailure.audioMessageFlagUnverified }
        let message = try message(transfer)
        guard let native = try object(message, "guid") as? String, !native.isEmpty else { throw SendVariantFailure.audioMessageFlagUnverified }
        let (selector, implementation) = try method(chat, "sendMessage:")
        typealias Call = @convention(c) (AnyObject, Selector, AnyObject) -> Void
        checkpoint("audio_send_requested")
        unsafeBitCast(implementation, to: Call.self)(chat, selector, message)
        return native
    }
}

public extension MessagesSendVariants {
    @MainActor
    static func syntheticNativeAudioFlag() -> Bool { MessagesNativeSession.syntheticFlag() }
    @MainActor
    static func syntheticForwardedNativeFlag(_ receiver: NSObject, selector: String) -> Bool { MessagesNativeSession.syntheticForwardedFlag(receiver, selector: selector) }
}
