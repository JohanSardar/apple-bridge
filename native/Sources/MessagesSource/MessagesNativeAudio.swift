import Foundation
import ObjectiveC
import Darwin
import SourceSupport

@MainActor
enum MessagesNativeAudio {
    private static func method(_ object: AnyObject, _ name: String) throws -> (Selector, IMP) {
        let selector = NSSelectorFromString(name)
        guard let type = object_getClass(object), let method = class_getInstanceMethod(type, selector) else { throw SendVariantFailure.audioMessageFlagUnverified }
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
        guard let message = unsafeBitCast(implementation, to: Call.self)(receiver, selector, body, nil, [transfer] as NSArray, 0x100005 | (1 << 21), nil)?.takeUnretainedValue(), try flag(message, "isAudioMessage") else { throw SendVariantFailure.audioMessageFlagUnverified }
        return message
    }
    static func syntheticFlag() -> Bool { (try? message("synthetic-transfer")) != nil }
    static func send(_ part: JSON, chat guid: String) throws -> String {
        guard !guid.isEmpty, dlopen("/System/Library/PrivateFrameworks/IMCore.framework/IMCore", RTLD_LAZY) != nil,
              let controllerType = NSClassFromString("IMDaemonController"), let registryType = NSClassFromString("IMChatRegistry"), let transferType = NSClassFromString("IMFileTransferCenter"),
              let controller = try object(controllerType, "sharedInstance"), let registry = try object(registryType, "sharedInstance"), let transfers = try object(transferType, "sharedInstance"),
              try flag(controller, "connectToDaemon") else { throw SendVariantFailure.audioMessageFlagUnverified }
        let deadline = Date().addingTimeInterval(5)
        var chat: AnyObject?
        repeat {
            chat = try object(registry, "existingChatWithGUID:", argument: guid as NSString)
            if chat != nil { break }
            RunLoop.current.run(until: Date().addingTimeInterval(0.05))
        } while Date() < deadline
        guard let chat, try object(chat, "guid") as? String == guid,
              let transfer = try object(transfers, "guidForNewOutgoingTransferWithLocalURL:", argument: NSURL(fileURLWithPath: part["path"].string)) as? String, !transfer.isEmpty else { throw SendVariantFailure.audioMessageFlagUnverified }
        let message = try message(transfer)
        guard let native = try object(message, "guid") as? String, !native.isEmpty else { throw SendVariantFailure.audioMessageFlagUnverified }
        let (selector, implementation) = try method(chat, "sendMessage:")
        typealias Call = @convention(c) (AnyObject, Selector, AnyObject) -> Void
        unsafeBitCast(implementation, to: Call.self)(chat, selector, message)
        return native
    }
}

public extension MessagesSendVariants {
    @MainActor
    static func syntheticNativeAudioFlag() -> Bool { MessagesNativeAudio.syntheticFlag() }
}
