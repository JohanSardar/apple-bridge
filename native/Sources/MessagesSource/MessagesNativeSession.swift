import Foundation
import ObjectiveC
import Darwin
import SourceSupport

@MainActor
enum MessagesNativeSession {
    private static func method(_ object: AnyObject, _ name: String) throws -> (Selector, IMP) {
        let selector = NSSelectorFromString(name)
        guard let type = object_getClass(object), let method = class_getInstanceMethod(type, selector) else {
            try? PrivateReport.write(["selector_digest": BodyDigest.hash(Data(name.utf8)), "selector_available": false], to: PrivateReport.directory.appendingPathComponent("messages-native-method-diagnostic.json"))
            throw SendVariantFailure.audioMessageFlagUnverified
        }
        return (selector, method_getImplementation(method))
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
              let controller = try object(controllerType, "sharedInstance"), let registry = try object(registryType, "sharedInstance") else { throw SendVariantFailure.audioMessageFlagUnverified }
        checkpoint("connection_requested")
        var connected = try flag(controller, "connectToDaemon")
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
        let deadline = Date().addingTimeInterval(5)
        var chat: AnyObject?
        repeat {
            chat = try object(registry, "existingChatWithGUID:", argument: guid as NSString)
            if chat != nil { break }
            RunLoop.current.run(until: Date().addingTimeInterval(0.05))
        } while Date() < deadline
        guard let chat, try object(chat, "guid") as? String == guid else { throw SendVariantFailure.audioMessageFlagUnverified }
        checkpoint("self_chat_resolved")
        return chat
    }
    static func deleteOwnedItem(_ guid: String, chat chatGUID: String) throws {
        let chat = try chat(chatGUID)
        guard let message = try object(chat, "messageForGUID:", argument: guid as NSString), try object(message, "guid") as? String == guid,
              let item = try object(message, "_imMessageItem"), try object(item, "guid") as? String == guid else { throw SourceFailure.conflict }
        let (selector, implementation) = try method(chat, "deleteIMMessageItems:")
        typealias Call = @convention(c) (AnyObject, Selector, NSArray) -> Void
        checkpoint("owned_item_delete_requested")
        unsafeBitCast(implementation, to: Call.self)(chat, selector, [item] as NSArray)
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
}
