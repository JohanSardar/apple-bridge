#if LAB
import Foundation
import ObjectiveC
import CryptoKit
import Darwin
import SourceSupport

@MainActor
public enum FocusPackRepair {
    public static func supportedShape(_ identifiers: [String]) -> Bool {
        let prefix = "is.workflow.actions."
        let expected = ["detect.dictionary", "getvalueforkey", "getvalueforkey", "conditional", "dnd.set", "conditional", "dnd.set", "conditional"].map { prefix + $0 }
        return identifiers == expected || identifiers == expected + [prefix + "output"]
    }
    public static func binding(_ input: JSON, output: String) throws -> JSON {
        let attachment: JSON
        if input["Type"].string == "Variable" { attachment = input["Variable"] }
        else { attachment = input }
        guard !output.isEmpty, attachment["WFSerializationType"].string == "WFTextTokenAttachment",
              attachment["Value"]["Type"].string == "ActionOutput", attachment["Value"]["OutputUUID"].string == output else { throw SourceFailure.invalid }
        return .object(["Type": .string("Variable"), "Variable": attachment])
    }

    public static func dateBinding(_ input: JSON, output: String) throws -> JSON {
        let attachment: JSON
        if input["WFSerializationType"].string == "WFTextTokenString" {
            guard input["Value"]["string"].string == "\u{fffc}", input["Value"]["attachmentsByRange"].object.count == 1 else { throw SourceFailure.invalid }
            attachment = input["Value"]["attachmentsByRange"]["{0, 1}"]
        } else {
            guard input["WFSerializationType"].string == "WFTextTokenAttachment" else { throw SourceFailure.invalid }
            attachment = input["Value"]
        }
        guard !output.isEmpty, attachment["Type"].string == "ActionOutput", attachment["OutputUUID"].string == output else { throw SourceFailure.invalid }
        return .object(["WFSerializationType": .string("WFTextTokenString"), "Value": .object(["string": .string("\u{fffc}"), "attachmentsByRange": .object(["{0, 1}": attachment])])])
    }

    private static func method(_ target: AnyObject, _ name: String, arguments: UInt32) throws -> (Selector, IMP) {
        let selector = NSSelectorFromString(name)
        guard let method = class_getInstanceMethod(object_getClass(target), selector), method_getNumberOfArguments(method) == arguments else { throw SourceFailure.unsupported }
        return (selector, method_getImplementation(method))
    }

    private static func object(_ target: AnyObject, _ name: String) throws -> AnyObject? {
        let (selector, implementation) = try method(target, name, arguments: 2)
        typealias Call = @convention(c) (AnyObject, Selector) -> Unmanaged<AnyObject>?
        return unsafeBitCast(implementation, to: Call.self)(target, selector)?.takeUnretainedValue()
    }

    private static func object(_ target: AnyObject, _ name: String, argument: AnyObject, errorArgument: Bool = false) throws -> AnyObject? {
        let (selector, implementation) = try method(target, name, arguments: errorArgument ? 4 : 3)
        if errorArgument {
            typealias Call = @convention(c) (AnyObject, Selector, AnyObject, UnsafeMutableRawPointer?) -> Unmanaged<AnyObject>?
            return unsafeBitCast(implementation, to: Call.self)(target, selector, argument, nil)?.takeUnretainedValue()
        }
        typealias Call = @convention(c) (AnyObject, Selector, AnyObject) -> Unmanaged<AnyObject>?
        return unsafeBitCast(implementation, to: Call.self)(target, selector, argument)?.takeUnretainedValue()
    }

    public static func database(testURL: URL? = nil) throws -> AnyObject {
        guard dlopen("/System/Library/PrivateFrameworks/WorkflowKit.framework/WorkflowKit", RTLD_LAZY) != nil,
              let databaseClass = NSClassFromString("WFDatabase"),
              let allocated = try object(databaseClass as AnyObject, "alloc") else { throw SourceFailure.unavailable }
        let database: AnyObject?
        if let testURL {
            let (selector, implementation) = try method(allocated, "initWithTestFileURL:error:", arguments: 4)
            typealias Initialize = @convention(c) (AnyObject, Selector, AnyObject, UnsafeMutableRawPointer?) -> Unmanaged<AnyObject>?
            database = unsafeBitCast(implementation, to: Initialize.self)(allocated, selector, testURL as NSURL, nil)?.takeRetainedValue()
        } else {
            let (selector, implementation) = try method(allocated, "initWithPersistenceMode:error:", arguments: 4)
            typealias Initialize = @convention(c) (AnyObject, Selector, UInt, UnsafeMutableRawPointer?) -> Unmanaged<AnyObject>?
            database = unsafeBitCast(implementation, to: Initialize.self)(allocated, selector, 0, nil)?.takeRetainedValue()
        }
        guard let database else { throw SourceFailure.unavailable }
        return database
    }

    public static func run() -> JSON {
        var phase = "framework"
        do {
            phase = "database_open"
            let database = try database()
            phase = "reference"
            guard let reference = try object(database, "uniqueVisibleReferenceForWorkflowName:", argument: "AB Focus" as NSString) else { throw SourceFailure.unavailable }
            phase = "record"
            guard let record = try object(database, "recordWithDescriptor:error:", argument: reference, errorArgument: true),
                  let actions = try object(record, "actions") as? [[String: Any]] else { throw SourceFailure.unavailable }
            let bytes = try JSONSerialization.data(withJSONObject: actions, options: [.sortedKeys])
            let value = try JSONDecoder().decode(JSON.self, from: bytes)
            let identifiers = value.array.map { $0["WFWorkflowActionIdentifier"].string }
            phase = "shape"
            guard supportedShape(identifiers),
                  value.array[1]["WFWorkflowActionParameters"]["WFDictionaryKey"].string == "on",
                  value.array[2]["WFWorkflowActionParameters"]["WFDictionaryKey"].string == "until",
                  value.array[3]["WFWorkflowActionParameters"]["WFControlFlowMode"] == .number(0),
                  value.array[5]["WFWorkflowActionParameters"]["WFControlFlowMode"] == .number(1),
                  value.array[7]["WFWorkflowActionParameters"]["WFControlFlowMode"] == .number(2) else { throw SourceFailure.invalid }
            let until = value.array[2]["WFWorkflowActionParameters"]["UUID"].string
            let original = value.array[3]["WFWorkflowActionParameters"]["WFInput"]
            let corrected = try binding(original, output: until)
            let originalDate = value.array[4]["WFWorkflowActionParameters"]["Time"]
            let correctedDate = try dateBinding(originalDate, output: until)
            var updated = value.array
            var action = updated[3].object
            var parameters = updated[3]["WFWorkflowActionParameters"].object
            parameters["WFInput"] = corrected
            action["WFWorkflowActionParameters"] = .object(parameters)
            updated[3] = .object(action)
            var timed = updated[4].object
            var timedParameters = updated[4]["WFWorkflowActionParameters"].object
            timedParameters["Time"] = correctedDate
            timed["WFWorkflowActionParameters"] = .object(timedParameters)
            updated[4] = .object(timed)
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.sortedKeys]
            let changed = try encoder.encode(JSON.array(updated))
            let needsRepair = original != corrected || originalDate != correctedDate
            if needsRepair {
                phase = "backup"
                let directory = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/AppleBridgeDev/focus-pack-backups")
                try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
                let digest = SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
                let backup = directory.appendingPathComponent(digest + ".json")
                if !FileManager.default.fileExists(atPath: backup.path) { try bytes.write(to: backup, options: .atomic) }
                guard try Data(contentsOf: backup) == bytes else { throw SourceFailure.conflict }
                try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: backup.path)
                phase = "update"
                let replacements = try JSONSerialization.jsonObject(with: changed) as AnyObject
                let (setSelector, setImplementation) = try method(record, "setActions:", arguments: 3)
                typealias SetActions = @convention(c) (AnyObject, Selector, AnyObject) -> Void
                unsafeBitCast(setImplementation, to: SetActions.self)(record, setSelector, replacements)
                let (saveSelector, saveImplementation) = try method(database, "saveRecord:withDescriptor:error:", arguments: 5)
                typealias Save = @convention(c) (AnyObject, Selector, AnyObject, AnyObject, UnsafeMutableRawPointer?) -> Bool
                guard unsafeBitCast(saveImplementation, to: Save.self)(database, saveSelector, record, reference, nil) else { throw SourceFailure.unavailable }
            }
            phase = "readback"
            guard let readback = try object(database, "recordWithDescriptor:error:", argument: reference, errorArgument: true),
                  let readActions = try object(readback, "actions"),
                  (readActions as? NSArray)?.isEqual(to: try JSONSerialization.jsonObject(with: changed) as? [Any] ?? []) == true else { throw SourceFailure.unavailable }
            return .object(["passed": .bool(true), "changed": .bool(needsRepair), "action_count": .number(Double(actions.count))])
        } catch {
            return .object(["passed": .bool(false), "phase": .string(phase)])
        }
    }
}
#endif
