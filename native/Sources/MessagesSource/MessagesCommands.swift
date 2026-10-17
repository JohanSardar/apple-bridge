import Foundation
import Decoders
import SourceSupport

public actor MessagesCommands {
    public static let operations = ["send", "reply", "react", "edit", "unsend", "mark_read", "mark_unread", "delete_message", "delete_chat", "create_group", "rename_group", "add_member", "remove_member", "leave_group"].map { "messages." + $0 }
    private let path: String
    private let labCommands: Bool
    public init(path: String?, labCommands: Bool) {
        self.path = path ?? FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/chat.db").path
        self.labCommands = labCommands
    }
    private var proofURL: URL { PrivateReport.directory.appendingPathComponent("messages-command-proofs.json") }
    private var osBuild: String { ProcessInfo.processInfo.operatingSystemVersionString }
    private func proofs() -> Set<String> {
        guard let data=try? Data(contentsOf:proofURL),let value=try? JSONDecoder().decode(JSON.self,from:data),value["os_build"].string==osBuild else {return []}
        return Set(value["routes"].array.map(\.string))
    }
    private func routeProof(_ route: JSON, kind: String) -> String {
        BodyDigest.hash(Data((route["kind"].string+"|"+route["chat"].string+"|"+route["handle"].string+"|"+route["service"].string).utf8))+":"+kind
    }
    public func capabilities() -> [String] { proofs().isEmpty ? [] : ["messages.send"] }
    public func operationCoverage() -> JSON {
        let proven = !proofs().isEmpty
        return .array(Self.operations.map { .object(["operation":.string($0),"proven":.bool($0=="messages.send"&&proven),"reason":.string($0=="messages.send" ? proven ? "proven_self_routes_and_parts_only" : "route_proof_required" : "pending_unlock")]) })
    }
    private func recordProof(_ route: JSON, parts: [JSON]) throws {
        var routes=proofs()
        for part in parts {routes.insert(routeProof(route,kind:part["kind"].string))}
        let data=try JSONEncoder().encode(JSON.object(["os_build":.string(osBuild),"routes":.array(routes.sorted().map(JSON.string))]))
        try FileManager.default.createDirectory(at:proofURL.deletingLastPathComponent(),withIntermediateDirectories:true,attributes:[.posixPermissions:0o700])
        try data.write(to:proofURL,options:.atomic)
        try FileManager.default.setAttributes([.posixPermissions:0o600],ofItemAtPath:proofURL.path)
    }
    private func tagged(_ text: String) -> Bool {
        text.range(of: #"\[ab-test [A-Za-z0-9_-]+\]"#, options: .regularExpression) != nil
    }
    private func verifyRoute(_ action: JSON, reader: MessagesReader) throws -> JSON {
        let args = action["messages"]
        let selfHandles = Set(try MessagesProbes.discoverSelfHandles().handles)
        guard !selfHandles.isEmpty else { throw SourceFailure.invalid }
        if !action["list_key"].string.isEmpty {
            let chat = action["list_key"].string
            let rows = try reader.query("SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?", [chat])
            guard !rows.isEmpty, rows.allSatisfy({ selfHandles.contains(MessagesProbes.normalize($0["id"])) }) else { throw SourceFailure.invalid }
            return .object(["kind": .string("chat_id_raw"), "chat": .string(chat)])
        }
        let handles = args["handles"].array.map(\.string)
        guard handles.count == 1, handles.allSatisfy({ selfHandles.contains(MessagesProbes.normalize($0)) }), args["service"].string == "iMessage" else { throw SourceFailure.unsupported }
        return .object(["kind": .string("imessage_buddy"), "handle": .string(handles[0]), "service": .string("iMessage")])
    }
    public func prepare(_ command: JSON) async throws -> Prepared {
        let action = command["action"]
        if action["op"].string != "messages.send" {return try await prepareAccessibility(command)}
        let args = action["messages"]
        guard args["text"].string.isEmpty || tagged(args["text"].string), !args["text"].string.isEmpty || !args["files"].array.isEmpty else { throw SourceFailure.invalid }
        for file in args["files"].array {
            let name=file["name"].string
            guard !name.isEmpty,name==URL(fileURLWithPath:name).lastPathComponent,name != ".",name != "..",let bytes=Data(base64Encoded:file["data"].string),bytes.count<=131072 else {throw SourceFailure.invalid}
        }
        let reader = try MessagesReader(path: path)
        let route = try verifyRoute(action, reader: reader)
        if !labCommands {
            let kinds = (args["text"].string.isEmpty ? [] : ["text"]) + (args["files"].array.isEmpty ? [] : ["attachment"])
            guard kinds.allSatisfy({proofs().contains(routeProof(route,kind:$0))}) else {throw SourceFailure.capabilityUnproven}
        }
        let maximum = try reader.maximum()
        let payload = try JSONEncoder().encode(args)
        let state = JSON.object(["route": route["kind"], "destination": route, "max_row_id": .number(Double(maximum)), "payload_fingerprint": .string(BodyDigest.hash(payload)), "command_id": command["command_id"], "candidate_ids": .array([.string(String(maximum)), .string(route["chat"].string)])])
        return Prepared(nativeState: state, fingerprint: command["request_hash"].string, action: action)
    }
    private func prepareAccessibility(_ command: JSON) async throws -> Prepared {
        let action=command["action"]
        let operation=try MessagesAction(operation:action["op"].string)
        let args=action["messages"]
        let reader=try MessagesReader(path:path)
        guard let row=try reader.message(action["native_key"].string),let item=try reader.messageObservations(row).first else {throw SourceFailure.capabilityUnproven}
        let target=try JSONDecoder().decode(JSON.self,from:JSONSerialization.data(withJSONObject:item.2,options:.sortedKeys))
        guard tagged(target["text"].string) else {throw SourceFailure.invalid}
        if !args["text"].string.isEmpty && !tagged(args["text"].string) {throw SourceFailure.invalid}
        var destination=action.object
        destination["list_key"] = .string(row["capture_chat_guid"])
        let route=try verifyRoute(.object(destination),reader:reader)
        if operation == .edit || operation == .unsend {
            let chat=try reader.query("SELECT service_name FROM chat WHERE guid=?",[row["capture_chat_guid"]]).first
            guard chat?["service_name"] == "iMessage", row.flag("is_from_me"),let sent=parseTimestamp(target["sent_at"].string) else {throw SourceFailure.unsupported}
            let age=Date().timeIntervalSince(sent)
            guard age >= 0, age < (operation == .edit ? 900 : 120), operation != .edit || target["edit_history"].array.count < 5 else {throw SourceFailure.unsupported}
        }
        for field in ["text","sent_at","edit_history","retracted_parts"] {
            let expected=command["expected_state"][field]
            if expected != .null && expected != target[field] {throw SourceFailure.conflict}
        }
        let prepared=Prepared(nativeState:.object(["route":.string("accessibility"),"destination":route,"target":target,"max_row_id":.number(Double(try reader.maximum())),"candidate_ids":.array([.string(row["guid"])])]),fingerprint:command["request_hash"].string,action:action)
        guard labCommands, await MessagesAccessibility.available else {throw SourceFailure.capabilityUnproven}
        try await MessagesAccessibility.validate(prepared)
        return prepared
    }
    public func validate(_ prepared: Prepared) async throws {
        if prepared.action["op"].string != "messages.send" {try await MessagesAccessibility.validate(prepared)}
    }
    private func dispatchAccessibility(_ prepared: Prepared) async throws -> JSON {
        let reader=try MessagesReader(path:path)
        let native=prepared.action["native_key"].string
        guard let before=try reader.message(native),let item=try reader.messageObservations(before).first else {throw SourceFailure.conflict}
        let current=try JSONDecoder().decode(JSON.self,from:JSONSerialization.data(withJSONObject:item.2,options:.sortedKeys))
        guard current==prepared.nativeState["target"] else {throw SourceFailure.conflict}
        let ui=try await MessagesAccessibility.perform(prepared)
        let deadline=ContinuousClock.now.advanced(by:.seconds(10))
        let operation=try MessagesAction(operation:prepared.action["op"].string)
        repeat {
            let row=try reader.message(native)
            let items=try row.map {try reader.messageObservations($0)} ?? []
            let data=try items.first.map {try JSONDecoder().decode(JSON.self,from:JSONSerialization.data(withJSONObject:$0.2,options:.sortedKeys))} ?? .null
            var observed=false
            switch operation {
            case .edit: observed=data["text"]==prepared.action["messages"]["text"] && data["edit_history"].array.count > current["edit_history"].array.count
            case .unsend: observed=data["retracted_at"] != .null || !data["retracted_parts"].array.isEmpty
            case .deleteMessage: observed=row == nil
            default: break
            }
            if observed {
                var evidence=ui.object
                evidence["source"] = .string("messages"); evidence["native_key"] = .string(native); evidence["collection"] = .string("messages.message"); evidence["observed"] = .bool(true); evidence["absent"] = .bool(row == nil); evidence["disposition"] = .string("locally_observed"); evidence["parts"] = .array([])
                return .object(evidence)
            }
            try await Task.sleep(for:.milliseconds(50))
        } while ContinuousClock.now < deadline
        var evidence=ui.object
        evidence["source"] = .string("messages"); evidence["observed"] = .bool(false); evidence["disposition"] = .string("indeterminate")
        return .object(evidence)
    }
    private func parts(_ prepared: Prepared) throws -> [JSON] {
        let args = prepared.action["messages"]
        var parts: [JSON] = []
        if !args["text"].string.isEmpty { parts.append(.object(["kind": .string("text"), "text": args["text"]])) }
        let root = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Messages/Attachments/AppleBridgeOutgoing", isDirectory: true)
        let directory = root.appendingPathComponent(BodyDigest.hash(Data(prepared.nativeState["command_id"].string.utf8)), isDirectory: true)
        if !args["files"].array.isEmpty {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        }
        for (index, file) in args["files"].array.enumerated() {
            let name = file["name"].string
            guard !name.isEmpty, name == URL(fileURLWithPath: name).lastPathComponent, name != ".", name != "..", let bytes = Data(base64Encoded: file["data"].string), bytes.count <= 131072 else { throw SourceFailure.invalid }
            let url = directory.appendingPathComponent("\(index)-" + name)
            try bytes.write(to: url, options: [.atomic, .withoutOverwriting])
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
            parts.append(.object(["kind": .string("attachment"), "path": .string(url.path), "name": .string(url.lastPathComponent), "digest": .string(BodyDigest.hash(bytes))]))
        }
        return parts
    }
    public func dispatch(_ prepared: Prepared) async throws -> JSON {
        if prepared.action["op"].string != "messages.send" {return try await dispatchAccessibility(prepared)}
        let reader = try MessagesReader(path: path)
        let route = try verifyRoute(prepared.action, reader: reader)
        guard route == prepared.nativeState["destination"] else { throw SourceFailure.conflict }
        let parts = try parts(prepared)
        var outcomes: [JSON] = []
        let deadline = ContinuousClock.now.advanced(by: .seconds(10))
        for (index, part) in parts.enumerated() {
            if ContinuousClock.now >= deadline {
                for remaining in index..<parts.count {outcomes.append(.object(["index":.number(Double(remaining)),"kind":parts[remaining]["kind"],"disposition":.string("not_dispatched")]))}
                break
            }
            let maximum = max(Int64(prepared.nativeState["max_row_id"].number), try reader.maximum())
            let error = await MessagesUI.send(part, route: route)
            var evidence: JSON = .object(["disposition": .string("indeterminate")])
            repeat {
                evidence = try correlate(part, route: route, maximum: maximum, reader: reader)
                if evidence["disposition"].string != "indeterminate" || evidence["ambiguous"].bool { break }
                try await Task.sleep(for: .milliseconds(50))
            } while ContinuousClock.now < deadline
            var value = evidence.object
            value["index"] = .number(Double(index))
            value["kind"] = part["kind"]
            value["apple_event_error"] = .number(Double(error))
            value["max_row_id"] = .number(Double(maximum))
            value["payload_fingerprint"] = .string(BodyDigest.hash(try JSONEncoder().encode(part)))
            outcomes.append(.object(value))
            if evidence["disposition"].string == "indeterminate" { break }
        }
        let result = aggregate(outcomes, prepared: prepared)
        if labCommands && ["locally_observed","delivered"].contains(result["disposition"].string) {try recordProof(route,parts:parts)}
        return result
    }
    private func correlate(_ part: JSON, route: JSON, maximum: Int64, reader: MessagesReader) throws -> JSON {
        let rows = try reader.messageRows(after: maximum, through: reader.maximum(), limit: 512).filter { $0.flag("is_from_me") }
        var matches: [SQLiteRow] = []
        for row in rows {
            if !route["chat"].string.isEmpty && row["capture_chat_guid"] != route["chat"].string { continue }
            if !route["handle"].string.isEmpty {
                let peers = try reader.query("SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?", [row["capture_chat_guid"]])
                guard peers.count == 1, MessagesProbes.normalize(peers[0]["id"]) == MessagesProbes.normalize(route["handle"].string) else { continue }
            }
            if part["kind"].string == "text" {
                let decoded = row.blobs["attributedBody"].map { TypedStream.decode($0).text } ?? row.values["text"]
                if decoded == part["text"].string { matches.append(row) }
            } else {
                let files = try reader.query("SELECT a.filename FROM attachment a JOIN message_attachment_join j ON j.attachment_id=a.ROWID JOIN message m ON m.ROWID=j.message_id WHERE m.guid=?", [row["guid"]])
                if files.contains(where: { URL(fileURLWithPath: $0["filename"]).lastPathComponent == part["name"].string }) { matches.append(row) }
            }
        }
        if matches.count > 1 { return .object(["disposition": .string("indeterminate"), "ambiguous": .bool(true), "candidate_count": .number(Double(matches.count))]) }
        if let row = matches.first {
            let disposition = row.integer("error") != 0 ? "delivery_failed" : row.flag("is_delivered") ? "delivered" : "locally_observed"
            return .object(["native_key": .string(row["guid"]), "disposition": .string(disposition), "observed": .bool(true), "error_code": .number(Double(row.integer("error")))])
        }
        let ghosts = rows.filter { $0["service"] == "SMS" && $0["capture_chat_guid"].isEmpty && ($0.values["text"] ?? "").isEmpty && $0.blobs["attributedBody"] == nil }
        if !ghosts.isEmpty { return .object(["disposition": .string("delivery_failed"), "reason": .string("empty_unjoined_sms"), "candidate_count": .number(Double(ghosts.count))]) }
        return .object(["disposition": .string("indeterminate")])
    }
    private func aggregate(_ parts: [JSON], prepared: Prepared) -> JSON {
        let dispositions = parts.map { $0["disposition"].string }
        let disposition = dispositions.contains("indeterminate") || dispositions.contains("not_dispatched") ? "indeterminate" : dispositions.contains("delivery_failed") ? "delivery_failed" : dispositions.allSatisfy { $0 == "delivered" } ? "delivered" : "locally_observed"
        return .object(["source": .string("messages"), "collection": .string("messages.message"), "native_key": parts.first(where: { !$0["native_key"].string.isEmpty })?["native_key"] ?? .null, "observed": .bool(parts.contains { $0["observed"].bool }), "disposition": .string(disposition), "parts": .array(parts), "fingerprint": .string(prepared.fingerprint), "baseline": prepared.nativeState])
    }
    public func resolve(_ evidence: JSON) async -> JSON {
        let saved=evidence["prepared"]
        guard saved["action"]["op"].string == "messages.send",let reader=try? MessagesReader(path:path) else {return evidence}
        let prepared=Prepared(nativeState:saved["native_state"],fingerprint:saved["request_hash"].string,action:saved["action"])
        let route=prepared.nativeState["destination"]
        let args=prepared.action["messages"]
        var requested:[JSON]=[]
        if !args["text"].string.isEmpty {requested.append(.object(["kind":.string("text"),"text":args["text"]]))}
        for (index,file) in args["files"].array.enumerated() {requested.append(.object(["kind":.string("attachment"),"name":.string("\(index)-"+file["name"].string)]))}
        var parts:[JSON]=[]
        for (index,part) in requested.enumerated() {
            let existing=evidence["parts"].array.first {$0["index"].number==Double(index)}
            if existing?["disposition"].string == "not_dispatched" {parts.append(existing!);continue}
            let maximum=existing?["max_row_id"].number ?? prepared.nativeState["max_row_id"].number
            guard let matched=try? correlate(part,route:route,maximum:Int64(maximum),reader:reader) else {return evidence}
            var value=matched.object
            value["index"] = .number(Double(index));value["kind"] = part["kind"];value["max_row_id"] = .number(maximum)
            parts.append(.object(value))
        }
        var resolved=aggregate(parts,prepared:prepared).object
        resolved["prepared"] = saved
        return .object(resolved)
    }
}

@UIActor
public enum MessagesUI {
    public static func send(_ part: JSON, route: JSON) -> Int {
        let content = part["kind"].string == "text" ? MessagesAppleScript.literal(part["text"].string) : "POSIX file " + MessagesAppleScript.literal(part["path"].string)
        if route["kind"].string == "chat_id_raw" {
            return MessagesAppleScript.execute("tell application id \"com.apple.MobileSMS\" to send \(content) to chat id \(MessagesAppleScript.literal(route["chat"].string))")
        }
        return MessagesAppleScript.execute("tell application id \"com.apple.MobileSMS\"\nset targetService to first service whose service type = iMessage\nsend \(content) to buddy \(MessagesAppleScript.literal(route["handle"].string)) of targetService\nend tell")
    }
}
