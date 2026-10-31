import Foundation
import Darwin
import Decoders
import SourceSupport

public actor MessagesCommands {
    public static let operations = ["send", "reply", "react", "edit", "unsend", "mark_read", "mark_unread", "delete_message", "delete_chat", "create_group", "rename_group", "add_member", "remove_member", "leave_group"].map { "messages." + $0 }
    private let path: String
    private let labCommands: Bool
    private let agentPolicy: URL?
    public init(path: String?, labCommands: Bool, agentPolicy: URL? = nil) {
        self.agentPolicy = agentPolicy
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
    private func proven(_ operation: String, routes: Set<String>) -> Bool {
        if operation == "messages.send" { return routes.contains { $0.hasSuffix(":text") || $0.hasSuffix(":attachment") } }
        return routes.contains { $0.hasSuffix(":" + operation) }
    }
    public func capabilities() -> [String] {
        let routes = proofs()
        return Self.operations.filter { proven($0, routes: routes) } + MessagesSendVariants.capabilities() + MessagesParity.coverage().filter { $0["proven"].bool }.map { $0["operation"].string }
    }
    public func operationCoverage() -> JSON {
        let routes = proofs()
        var reasons: [String: [String]] = [:]
        let reportURL = PrivateReport.directory.appendingPathComponent("messages-selftest.json")
        if let data = try? Data(contentsOf: reportURL), let report = try? JSONDecoder().decode(JSON.self, from: data), report["os_build"].string == osBuild {
            for step in report["steps"].array where !step["proven"].bool && !step["operation"].string.isEmpty && !step["reason"].string.isEmpty {
                reasons[step["operation"].string, default: []].append(step["reason"].string)
            }
        }
        return .array(Self.operations.map { operation in
            let supported = proven(operation, routes: routes)
            return .object(["operation": .string(operation), "proven": .bool(supported), "reason": .string(supported ? "proven_routes_and_parts_only" : reasons[operation].map { Set($0).sorted().joined(separator: ",") } ?? "route_proof_required")])
        } + MessagesSendVariants.coverage() + MessagesParity.coverage())
    }
    private func recordProof(_ route: JSON, parts: [JSON]) throws {
        var routes=proofs()
        for part in parts {routes.insert(routeProof(route,kind:part["kind"].string))}
        let data=try JSONEncoder().encode(JSON.object(["os_build":.string(osBuild),"routes":.array(routes.sorted().map(JSON.string))]))
        try FileManager.default.createDirectory(at:proofURL.deletingLastPathComponent(),withIntermediateDirectories:true,attributes:[.posixPermissions:0o700])
        try data.write(to:proofURL,options:.atomic)
        try FileManager.default.setAttributes([.posixPermissions:0o600],ofItemAtPath:proofURL.path)
    }
    public func invalidateProof(_ operation: String) throws {
        let kinds = operation == "messages.send" ? ["text", "attachment"] : [operation]
        let routes = proofs().filter { route in !kinds.contains(where: { route.hasSuffix(":" + $0) }) }
        let data = try JSONEncoder().encode(JSON.object(["os_build": .string(osBuild), "routes": .array(routes.sorted().map(JSON.string))]))
        try FileManager.default.createDirectory(at: proofURL.deletingLastPathComponent(), withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        try data.write(to: proofURL, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: proofURL.path)
    }
    private func tagged(_ text: String) -> Bool {
        text.range(of: #"\[ab-test [A-Za-z0-9_-]+\]"#, options: .regularExpression) != nil
    }
    private func approvedHandles() throws -> Set<String> {
        var handles = Set(try MessagesProbes.discoverSelfHandles().handles)
        if labCommands {
            let url = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".config/apple-bridge/test-contacts.env")
            if let text = try? String(contentsOf: url, encoding: .utf8) {
                let keys: Set<String> = ["REMY_IMESSAGE_TEST_HANDLE", "REMY_IMESSAGE_TEST_EMAIL", "BEN_GROUP_TEST_HANDLE"]
                for line in text.components(separatedBy: .newlines) {
                    let parts = line.replacingOccurrences(of: "export ", with: "").split(separator: "=", maxSplits: 1).map(String.init)
                    if parts.count == 2, keys.contains(parts[0].trimmingCharacters(in: .whitespaces)) {
                        handles.insert(MessagesProbes.normalize(parts[1].trimmingCharacters(in: CharacterSet(charactersIn: " \"'"))))
                    }
                }
            }
        }
        return handles
    }
    private func prepareGroup(_ command: JSON) throws -> Prepared {
        let action = command["action"]
        let args = action["messages"]
        let handles = args["handles"].array.map { MessagesProbes.normalize($0.string) }
        let interactive = try CommandOrigin(command) == .interactive
        let allowed = interactive ? Set<String>() : try approvedHandles()
        let route = JSON.object(["kind": .string("group_addresses"), "handle": .string(handles.sorted().joined(separator: ",")), "service": args["service"]])
        guard handles.count >= 2, Set(handles).count == handles.count, interactive || tagged(args["text"].string), ["iMessage", "SMS", "RCS"].contains(args["service"].string) else { throw SourceFailure.invalid }
        guard args["name"].string.isEmpty else { throw SourceFailure.unsupported }
        guard labCommands ? (interactive || handles.allSatisfy({ allowed.contains($0) })) : proofs().contains(routeProof(route, kind: "messages.create_group")) else { throw SourceFailure.capabilityUnproven }
        let reader = try MessagesReader(path: path)
        let maximumChat = try reader.query("SELECT COALESCE(MAX(ROWID),0) AS maximum FROM chat", []).first?["maximum"] ?? "0"
        return Prepared(nativeState: .object(["max_chat_row_id": .number(Double(maximumChat) ?? 0), "max_row_id": .number(Double(try reader.maximum())), "handles": .array(handles.map(JSON.string)), "destination": route]), fingerprint: command["request_hash"].string, action: action)
    }
    private func dispatchGroup(_ prepared: Prepared) async throws -> JSON {
        let reader = try MessagesReader(path: path)
        let args = prepared.action["messages"]
        let handles = prepared.nativeState["handles"].array.map(\.string)
        try await MessagesAccessibility.createGroup(args)
        let error = 0
        let deadline = ContinuousClock.now.advanced(by: .seconds(10))
        repeat {
            let rows = try reader.query("SELECT m.guid,c.guid AS chat_key,c.style AS style,c.ROWID AS chat_row_id FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE m.ROWID>? AND m.is_from_me=1 AND (m.text=? OR instr(m.attributedBody,CAST(? AS BLOB))>0)", [String(Int64(prepared.nativeState["max_row_id"].number)), args["text"].string, args["text"].string])
            if rows.count == 1, let row = rows.first {
                let peers = try reader.query("SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?", [row["chat_key"]])
                let actual = Set(peers.map { MessagesProbes.normalize($0["id"]) })
                guard actual == Set(handles), row["style"] == "43" else {
                    return .object(["source": .string("messages"), "collection": .string("messages.message"), "native_key": .string(row["guid"]), "chat_key": .string(row["chat_key"]), "participant_count": .number(Double(actual.count)), "group_style": .string(row["style"]), "observed": .bool(false), "disposition": .string("indeterminate"), "reason": .string("participants_not_distinct_group")])
                }
                if labCommands { try recordProof(prepared.nativeState["destination"], parts: [.object(["kind": .string("messages.create_group")])]) }
                return .object(["source": .string("messages"), "collection": .string("messages.message"), "native_key": .string(row["guid"]), "chat_key": .string(row["chat_key"]), "created_by_run": .bool((Double(row["chat_row_id"]) ?? 0) > prepared.nativeState["max_chat_row_id"].number), "observed": .bool(true), "disposition": .string("locally_observed"), "apple_event_error": .number(Double(error)), "parts": .array([])])
            }
            try await Task.sleep(for: .milliseconds(50))
        } while ContinuousClock.now < deadline
        return .object(["source": .string("messages"), "observed": .bool(false), "disposition": .string("indeterminate"), "apple_event_error": .number(Double(error))])
    }
    private func verifyRoute(_ action: JSON, reader: MessagesReader, interactive: Bool) throws -> JSON {
        let args = action["messages"]
        let selfHandles = interactive ? Set<String>() : try approvedHandles()
        guard interactive || !selfHandles.isEmpty else { throw SourceFailure.invalid }
        if !action["list_key"].string.isEmpty {
            let chat = action["list_key"].string
            let rows = try reader.query("SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?", [chat])
            let route = JSON.object(["kind": .string("chat_id_raw"), "chat": .string(chat)])
            let known = proofs()
            let provenRoute = action["op"].string == "messages.send" ? ["text", "attachment"].contains { known.contains(routeProof(route, kind: $0)) } : known.contains(routeProof(route, kind: action["op"].string))
            guard !rows.isEmpty, interactive || rows.allSatisfy({ selfHandles.contains(MessagesProbes.normalize($0["id"])) }) || provenRoute else { throw SourceFailure.invalid }
            return route
        }
        let handles = args["handles"].array.map(\.string)
        guard handles.count == 1, interactive || handles.allSatisfy({ selfHandles.contains(MessagesProbes.normalize($0)) }), args["service"].string == "iMessage" else { throw SourceFailure.unsupported }
        let chats = try reader.query("SELECT c.guid,h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.service_name='iMessage' AND (SELECT count(*) FROM chat_handle_join x WHERE x.chat_id=c.ROWID)=1 ORDER BY c.ROWID DESC", [])
        if let chat = chats.first(where: { MessagesProbes.normalize($0["id"]) == MessagesProbes.normalize(handles[0]) }) {
            return .object(["kind": .string("chat_id_raw"), "chat": .string(chat["guid"])])
        }
        return .object(["kind": .string("imessage_buddy"), "handle": .string(handles[0]), "service": .string("iMessage")])
    }
    private func validateResend(_ action: JSON, route: JSON, reader: MessagesReader) throws {
        let native = action["messages"]["resend_of"].string
        guard !native.isEmpty else { return }
        guard let failed = try reader.message(native), failed.flag("is_from_me"), failed.integer("error") != 0, !route["chat"].string.isEmpty, failed["capture_chat_guid"] == route["chat"].string else { throw SourceFailure.conflict }
    }
    private func validateAgentPolicy(_ action: JSON, route: JSON) throws {
        guard let agentPolicy, FileManager.default.fileExists(atPath: agentPolicy.path) else { return }
        let policy = try JSONDecoder().decode(JSON.self, from: Data(contentsOf: agentPolicy))
        guard action["messages"]["variant"] == .null, action["op"].string == "messages.send", route["kind"].string == "chat_id_raw", route["chat"].string == policy["chat_guid"].string, action["messages"]["text"].string == policy["text"].string, action["messages"]["files"].array.isEmpty, !FileManager.default.fileExists(atPath: agentPolicy.path + ".used") else { throw SourceFailure.invalid }
    }
    public func prepare(_ command: JSON) async throws -> Prepared {
        let interactive = try CommandOrigin(command) == .interactive
        let action = command["action"]
        if let agentPolicy, FileManager.default.fileExists(atPath: agentPolicy.path), action["op"].string != "messages.send" { throw SourceFailure.invalid }
        if action["op"].string == "messages.keep_audio" { throw SendVariantFailure.keptReadbackUnavailable }
        if MessagesParity.operations.contains(action["op"].string) { return try await MessagesParity.prepare(command, path: path) }
        if action["op"].string == "messages.create_group" { return try prepareGroup(command) }
        if action["op"].string != "messages.send" {return try await prepareAccessibility(command)}
        let args = action["messages"]
        guard interactive || args["text"].string.isEmpty || tagged(args["text"].string), !args["text"].string.isEmpty || !args["files"].array.isEmpty else { throw SourceFailure.invalid }
        for file in args["files"].array {
            let name=file["name"].string
            guard !name.isEmpty,name==URL(fileURLWithPath:name).lastPathComponent,name != ".",name != "..",file["data"].string.isEmpty else {throw SourceFailure.invalid}
        }
        let reader = try MessagesReader(path: path)
        let route = try verifyRoute(action, reader: reader, interactive: interactive)
        if args["variant"] != .null {
            try MessagesSendVariants.validate(args)
            guard labCommands || MessagesSendVariants.proven(MessagesSendVariants.operation(args) ?? "",route:route,args:args) else { throw SourceFailure.capabilityUnproven }
        }
        try validateResend(action, route: route, reader: reader)
        try validateAgentPolicy(action, route: route)
        if !labCommands {
            let kinds = (args["text"].string.isEmpty ? [] : ["text"]) + (args["files"].array.isEmpty ? [] : ["attachment"])
            guard kinds.allSatisfy({proofs().contains(routeProof(route,kind:$0))}) else {throw SourceFailure.capabilityUnproven}
        }
        let maximum = try reader.maximum()
        let payload = try JSONEncoder().encode(args)
        let state = JSON.object(["interactive": .bool(interactive), "route": route["kind"], "destination": route, "max_row_id": .number(Double(maximum)), "payload_fingerprint": .string(BodyDigest.hash(payload)), "command_id": command["command_id"], "expires_at": command["expires_at"], "candidate_ids": .array([.string(String(maximum)), .string(route["chat"].string)])])
        return Prepared(nativeState: state, fingerprint: command["request_hash"].string, action: action)
    }
    private func prepareAccessibility(_ command: JSON) async throws -> Prepared {
        let interactive = try CommandOrigin(command) == .interactive
        let action=command["action"]
        let operation=try MessagesAction(operation:action["op"].string)
        let args=action["messages"]
        let reader=try MessagesReader(path:path)
        if [.markRead, .markUnread, .deleteChat].contains(operation) { return try await prepareConversation(command, reader: reader, interactive: interactive) }
        if [.renameGroup, .addMember, .removeMember, .leaveGroup].contains(operation) {
            guard let chat = try reader.query("SELECT guid,style,service_name FROM chat WHERE guid=?",[action["native_key"].string]).first else { throw SourceFailure.capabilityUnproven }
            let peers = try reader.query("SELECT handle_id FROM chat_handle_join WHERE chat_id=(SELECT ROWID FROM chat WHERE guid=?)",[chat["guid"]])
            guard chat["style"] == "43", chat["service_name"] == "iMessage", peers.count >= 2 else { throw SourceFailure.unsupported }
            guard ![.removeMember, .leaveGroup].contains(operation) || peers.count >= 3 else { throw SourceFailure.unsupported }
            throw SourceFailure.capabilityUnproven
        }
        guard let row=try reader.message(action["native_key"].string),let item=try reader.messageObservations(row).first else {throw SourceFailure.capabilityUnproven}
        let target=try JSONDecoder().decode(JSON.self,from:JSONSerialization.data(withJSONObject:item.2,options:.sortedKeys))
        guard target["deleted_at"] == .null else { throw SourceFailure.unsupported }
        guard interactive || tagged(target["text"].string) else {throw SourceFailure.invalid}
        if !interactive && !args["text"].string.isEmpty && !tagged(args["text"].string) {throw SourceFailure.invalid}
        var destination=action.object
        destination["list_key"] = .string(row["capture_chat_guid"])
        let route=try verifyRoute(.object(destination),reader:reader,interactive:interactive)
        if operation == .edit || operation == .unsend {
            guard MessageActionWindow(service: row["service"], isFromMe: row.flag("is_from_me"), isMessage: row.integer("item_type") == 0, sentAt: parseTimestamp(target["sent_at"].string), edits: target["edit_history"].array.count)?.allows(edit: operation == .edit) == true else {throw SourceFailure.unsupported}
        }
        for field in ["text","edit_history","retracted_parts"] {
            let expected=command["expected_state"][field]
            if expected != .null && expected != target[field] {
                let delta = field == "sent_at" ? parseTimestamp(expected.string).flatMap { first in parseTimestamp(target[field].string).map { first.timeIntervalSince($0) } } : nil
                try? PrivateReport.write(["stage": "expected_state", "field": field, "delta_seconds": delta.map { $0 as Any } ?? NSNull(), "expected_length": expected.string.count, "actual_length": target[field].string.count], to: PrivateReport.validate(PrivateReport.directory.appendingPathComponent("messages-prepare-diagnostic.json").path))
                throw SourceFailure.conflict
            }
        }
        let own = Set(try MessagesProbes.discoverSelfHandles().handles)
        let peers = try reader.query("SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?", [row["capture_chat_guid"]])
        let selfChat = !peers.isEmpty && peers.allSatisfy { own.contains(MessagesProbes.normalize($0["id"])) }
        let reactions = try reader.query("SELECT guid,associated_message_type,associated_message_emoji FROM message WHERE (is_from_me=1 OR ?='1') AND associated_message_guid LIKE ?", [selfChat ? "1" : "0", "%" + row["guid"]]).map { item in JSON.object(["guid": .string(item["guid"]), "type": .string(item["associated_message_type"]), "emoji": .string(item["associated_message_emoji"])]) }
        let prepared=Prepared(nativeState:.object(["self_chat": .bool(selfChat), "reactions": .array(reactions), "route":.string("accessibility"),"destination":route,"target":target,"max_row_id":.number(Double(try reader.maximum())),"candidate_ids":.array([.string(row["guid"])])]),fingerprint:command["request_hash"].string,action:action)
        guard labCommands || proofs().contains(routeProof(route, kind: action["op"].string)), await MessagesAccessibility.available else {throw SourceFailure.capabilityUnproven}
        try await MessagesAccessibility.openMessage(row["guid"])
        if try await MessagesAccessibility.acknowledgeDeletedInfo() { try await MessagesAccessibility.openMessage(row["guid"]) }
        try await MessagesAccessibility.validate(prepared)
        return prepared
    }
    private func latestConversationDate(_ rows: [SQLiteRow]) -> Double {
        rows.compactMap { Double($0["date"]) }.map { $0 > 1e12 ? $0 / 1e9 : $0 }.max() ?? 0
    }
    private func prepareConversation(_ command: JSON, reader: MessagesReader, interactive: Bool) async throws -> Prepared {
        let action = command["action"]
        let chat = action["native_key"].string
        guard let conversation = try reader.query("SELECT ROWID AS row_position,* FROM chat WHERE guid=?", [chat]).first else { throw SourceFailure.capabilityUnproven }
        let rows = try reader.query("SELECT m.guid,m.date FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE c.guid=? AND NOT EXISTS (SELECT 1 FROM chat_recoverable_message_join r WHERE r.message_id=m.ROWID AND r.chat_id=c.ROWID) ORDER BY m.ROWID DESC", [chat])
        guard let last = rows.first, let row = try reader.message(last["guid"]), let item = try reader.messageObservations(row).first else { throw SourceFailure.capabilityUnproven }
        let target = try JSONDecoder().decode(JSON.self, from: JSONSerialization.data(withJSONObject: item.2))
        if !interactive {
            guard tagged(target["text"].string) else { throw SourceFailure.invalid }
            for item in rows {
                guard let message = try reader.message(item["guid"]), let data = try reader.messageObservations(message).first, let text = data.2["text"] as? String, tagged(text) else { throw SourceFailure.unsupported }
            }
        }
        var destination = action.object
        destination["list_key"] = .string(chat)
        let route = try verifyRoute(.object(destination), reader: reader, interactive: interactive)
        if action["op"].string == "messages.delete_chat", !interactive {
            guard let run = target["text"].string.range(of: #"\[ab-test ([A-Za-z0-9_-]+)\]"#, options: .regularExpression) else { throw SourceFailure.invalid }
            let tag = String(target["text"].string[run])
            let id = String(tag.dropFirst(9).dropLast())
            let ledger = PrivateReport.directory.appendingPathComponent("messages-command-cleanup/" + id + ".json")
            let registration = try JSONDecoder().decode(JSON.self, from: Data(contentsOf: ledger))
            guard registration["run_id"].string == id, registration["created_chat_key"].string == chat, registration["created_by_run"].bool, conversation.integer("row_position") > Int64(registration["baseline_max_chat_row_id"].number) else { throw SourceFailure.unsupported }
            for item in rows {
                guard let message = try reader.message(item["guid"]), let data = try reader.messageObservations(message).first, (data.2["text"] as? String)?.contains(tag) == true else { throw SourceFailure.invalid }
            }
        }
        guard labCommands || proofs().contains(routeProof(route, kind: action["op"].string)), await MessagesAccessibility.available else { throw SourceFailure.capabilityUnproven }
        let prepared = Prepared(nativeState: .object(["interactive": .bool(interactive), "conversation_members": .array(rows.map { .string($0["guid"]) }), "conversation_target": .bool(true), "target": target, "destination": route, "route": .string("accessibility"), "last_read_timestamp": .string(conversation["last_read_message_timestamp"]), "latest_message_date": .string(String(latestConversationDate(rows))), "candidate_ids": .array([.string(chat)])]), fingerprint: command["request_hash"].string, action: action)
        await MessagesUISession.activate()
        try await MessagesAccessibility.validate(prepared)
        return prepared
    }
    private func dispatchConversation(_ prepared: Prepared) async throws -> JSON {
        let reader = try MessagesReader(path: path)
        let chat = prepared.action["native_key"].string
        var destination = prepared.action.object
        destination["list_key"] = .string(chat)
        guard try verifyRoute(.object(destination), reader: reader, interactive: prepared.nativeState["interactive"].bool) == prepared.nativeState["destination"] else { throw SourceFailure.conflict }
        guard let current = try reader.query("SELECT last_read_message_timestamp FROM chat WHERE guid=?", [chat]).first, current["last_read_message_timestamp"] == prepared.nativeState["last_read_timestamp"].string else { throw SourceFailure.conflict }
        let rows = try reader.query("SELECT m.guid,m.date FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE c.guid=? AND NOT EXISTS (SELECT 1 FROM chat_recoverable_message_join r WHERE r.message_id=m.ROWID AND r.chat_id=c.ROWID) ORDER BY m.ROWID DESC", [chat])
        guard JSON.array(rows.map { .string($0["guid"]) }) == prepared.nativeState["conversation_members"], String(latestConversationDate(rows)) == prepared.nativeState["latest_message_date"].string else { throw SourceFailure.conflict }
        let ui = try await MessagesAccessibility.perform(prepared)
        let deadline = ContinuousClock.now.advanced(by: .seconds(10))
        repeat {
            let current = try reader.query("SELECT last_read_message_timestamp FROM chat WHERE guid=?", [chat]).first
            let operation = prepared.action["op"].string
            let rawLatest = Double(prepared.nativeState["latest_message_date"].string) ?? 0
            let rawTimestamp = Double(current?["last_read_message_timestamp"] ?? "0") ?? 0
            let latest = rawLatest > 1e12 ? rawLatest / 1e9 : rawLatest
            let timestamp = rawTimestamp > 1e12 ? rawTimestamp / 1e9 : rawTimestamp
            let recoverable = try reader.recoverableChat(chat)
            let deleted = current == nil || recoverable
            let observed = operation == "messages.delete_chat" ? deleted : current != nil && latest > 0 && (operation == "messages.mark_read" ? timestamp >= latest : timestamp < latest) && current?["last_read_message_timestamp"] != prepared.nativeState["last_read_timestamp"].string
            if observed {
                var evidence = ui.object
                evidence["source"] = .string("messages")
                evidence["collection"] = .string("messages.chat")
                evidence["native_key"] = .string(chat)
                evidence["observed"] = .bool(true)
                evidence["absent"] = .bool(current == nil)
                evidence["disposition"] = .string("locally_observed")
                return .object(evidence)
            }
            try await Task.sleep(for: .milliseconds(50))
        } while ContinuousClock.now < deadline
        return .object(["source": .string("messages"), "observed": .bool(false), "disposition": .string("indeterminate"), "reason": .string("conversation_effect_unproven"), "ui": ui])
    }
    public func validate(_ prepared: Prepared) async throws {
        if MessagesParity.operations.contains(prepared.action["op"].string) { try await MessagesParity.validate(prepared); return }
        if !["messages.send", "messages.create_group"].contains(prepared.action["op"].string) {try await MessagesAccessibility.validate(prepared)}
    }
    private func dispatchAccessibility(_ prepared: Prepared) async throws -> JSON {
        if prepared.nativeState["conversation_target"].bool { return try await dispatchConversation(prepared) }
        let reader=try MessagesReader(path:path)
        let native=prepared.action["native_key"].string
        guard let before=try reader.message(native),let item=try reader.messageObservations(before).first else {throw SourceFailure.conflict}
        let current=try JSONDecoder().decode(JSON.self,from:JSONSerialization.data(withJSONObject:item.2,options:.sortedKeys))
        for field in ["text", "is_from_me", "edit_history", "retracted_parts", "retracted_at", "deleted_at"] {
            guard current[field] == prepared.nativeState["target"][field] else { throw SourceFailure.conflict }
        }
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
            case .reply:
                let replies = try reader.query("SELECT guid FROM message WHERE ROWID>? AND is_from_me=1 AND thread_originator_guid=? AND (text=? OR instr(attributedBody,CAST(? AS BLOB))>0)", [String(Int64(prepared.nativeState["max_row_id"].number)), native, prepared.action["messages"]["text"].string, prepared.action["messages"]["text"].string])
                observed = replies.count == 1
                if observed, let key = replies.first?["guid"] {
                    return .object(["source": .string("messages"), "native_key": .string(key), "collection": .string("messages.message"), "observed": .bool(true), "disposition": .string("locally_observed"), "parts": .array([]), "ui": ui])
                }
            case .react:
                let args = prepared.action["messages"]
                let kinds = ["love", "like", "dislike", "laugh", "emphasize", "question", "emoji"]
                guard let index = kinds.firstIndex(of: args["reaction"].string) else { throw SourceFailure.invalid }
                let type = (args["remove"].bool ? 3000 : 2000) + index
                let reactions = try reader.query("SELECT guid,associated_message_type,associated_message_emoji FROM message WHERE (is_from_me=1 OR ?='1') AND associated_message_guid LIKE ? AND associated_message_type=?", [prepared.nativeState["self_chat"].bool ? "1" : "0", "%" + native, String(type)]).filter { item in
                    let signature = JSON.object(["guid": .string(item["guid"]), "type": .string(item["associated_message_type"]), "emoji": .string(item["associated_message_emoji"])])
                    return !prepared.nativeState["reactions"].array.contains(signature) && (args["reaction"].string != "emoji" || args["remove"].bool || item["associated_message_emoji"] == args["emoji"].string)
                }
                observed = reactions.count == 1
                if observed, let key = reactions.first?["guid"] {
                    return .object(["source": .string("messages"), "native_key": .string(key), "collection": .string("messages.reaction"), "observed": .bool(true), "disposition": .string("locally_observed"), "parts": .array([]), "ui": ui])
                }
            case .deleteMessage: observed=row == nil || data["deleted_at"] != .null
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
            guard !name.isEmpty, name == URL(fileURLWithPath: name).lastPathComponent, name != ".", name != "..", file["data"].string.isEmpty else { throw SourceFailure.invalid }
            let url = directory.appendingPathComponent("\(index)-" + name)
            let source=URL(fileURLWithPath:file["path"].string).resolvingSymlinksInPath()
            let parent=try FileManager.default.attributesOfItem(atPath:source.deletingLastPathComponent().path)
            let attributes=try FileManager.default.attributesOfItem(atPath:source.path)
            guard source.deletingLastPathComponent().lastPathComponent=="message-uploads",(parent[.posixPermissions] as? NSNumber)?.intValue==0o700,(parent[.ownerAccountID] as? NSNumber)?.uint32Value==getuid(),(attributes[.ownerAccountID] as? NSNumber)?.uint32Value==getuid(),attributes[.type] as? FileAttributeType == .typeRegular else {throw SourceFailure.invalid}
            try FileManager.default.copyItem(at:source,to:url)
            guard try BlobDigest.file(url)==file["digest"].string else {throw SourceFailure.invalid}
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
            parts.append(.object(["kind": .string("attachment"), "path": .string(url.path), "name": .string(url.lastPathComponent), "digest": file["digest"]]))
        }
        return parts
    }
    public func dispatch(_ prepared: Prepared) async throws -> JSON {
        if prepared.action["messages"]["variant"] != .null {
            let reader=try MessagesReader(path:path),route=try verifyRoute(prepared.action,reader:reader,interactive:prepared.nativeState["interactive"].bool)
            guard route==prepared.nativeState["destination"],labCommands || MessagesSendVariants.proven(MessagesSendVariants.operation(prepared.action["messages"]) ?? "",route:route,args:prepared.action["messages"]) else { throw SourceFailure.capabilityUnproven }
            return try await MessagesSendVariants.dispatch(prepared,path:path)
        }
        if MessagesParity.operations.contains(prepared.action["op"].string) { return try await MessagesParity.dispatch(prepared, path: path) }
        if prepared.action["op"].string == "messages.create_group" { return try await dispatchGroup(prepared) }
        if prepared.action["op"].string != "messages.send" {
            let result = try await dispatchAccessibility(prepared)
            if labCommands && result["observed"].bool { try recordProof(prepared.nativeState["destination"], parts: [.object(["kind": prepared.action["op"]])]) }
            return result
        }
        let reader = try MessagesReader(path: path)
        let route = try verifyRoute(prepared.action, reader: reader, interactive: prepared.nativeState["interactive"].bool)
        guard route == prepared.nativeState["destination"] else { throw SourceFailure.conflict }
        try validateResend(prepared.action, route: route, reader: reader)
        try validateAgentPolicy(prepared.action, route: route)
        if let agentPolicy, FileManager.default.fileExists(atPath: agentPolicy.path) {
            try Data(prepared.nativeState["command_id"].string.utf8).write(to: URL(fileURLWithPath: agentPolicy.path + ".used"), options: .withoutOverwriting)
        }
        let parts: [JSON]
        do {parts=try self.parts(prepared)}
        catch {
            let native=error as NSError
            return .object(["source":.string("messages"),"disposition":.string("failed_no_effect"),"observed":.bool(false),"reason":.string("staging_failed"),"error_domain":.string(native.domain),"error_code":.number(Double(native.code)),"fingerprint":.string(prepared.fingerprint)])
        }
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
            if evidence["disposition"].string == "indeterminate" {
                if index+1<parts.count {for remaining in (index+1)..<parts.count {outcomes.append(.object(["index":.number(Double(remaining)),"kind":parts[remaining]["kind"],"disposition":.string("not_dispatched"),"reason":.string("prior_part_indeterminate")]))}}
                break
            }
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
    public static func execute(_ script: String) -> Int { MessagesAppleScript.execute(script) }
    public static func send(_ part: JSON, route: JSON) -> Int {
        let content = part["kind"].string == "text" ? MessagesAppleScript.literal(part["text"].string) : "POSIX file " + MessagesAppleScript.literal(part["path"].string)
        if route["kind"].string == "chat_id_raw" {
            return MessagesAppleScript.execute("tell application id \"com.apple.MobileSMS\" to send \(content) to chat id \(MessagesAppleScript.literal(route["chat"].string))")
        }
        return MessagesAppleScript.execute("tell application id \"com.apple.MobileSMS\"\nset targetService to first service whose service type = iMessage\nsend \(content) to buddy \(MessagesAppleScript.literal(route["handle"].string)) of targetService\nend tell")
    }
}
