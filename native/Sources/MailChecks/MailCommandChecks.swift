import Foundation
import MailSource
import SourceSupport

actor MailFake {
    var calls: [String] = []
    let evidence: String
    let lag: Int
    let identity: String
    var readbacks = 0
    init(_ evidence: String, lag: Int = 0, identity: String = "123") { self.evidence = evidence; self.lag = lag; self.identity = identity }
    func run(_ body: String) -> String {
        calls.append(body)
        if body.hasPrefix("set matched to"), !body.contains("return count of matched") { return identity }
        if calls.count == 1 && (body.hasPrefix("return") || body.contains("return count of matched")) { return "0" }
        if body.hasPrefix("return") || body.contains("return (") || body.contains("return count of matched") {
            readbacks += 1
            return readbacks <= lag ? "" : evidence
        }
        return ""
    }
    func scripts() -> [String] { calls }
}
private actor MailCommandSink: ObservationSink {
    var messages: [JSON] = []
    func sourceState(source: String, state: String, coverage: JSON, capabilities: [String], reason: String) async throws {}
    func emit(source: String, generation: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws { throw SourceFailure.corrupt }
    func emitCheckpoint(source: String, generation: String, checkpoint: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {
        try require(source == "mail" && !generation.isEmpty && !checkpoint.isEmpty && observations.count == 1 && scope.isEmpty && completionEvidence.isEmpty && live)
        messages = observations
    }
}
func checkMailCommandCapture(_ path: String) async throws {
    let fake = MailFake("true")
    let source = MailSource(path: path, script: { await fake.run($0) })
    try require(Set(await source.open().capabilities) == Set(MailCommands.operations))
    _ = try await source.snapshot()
    let prepared = try await source.prepare(.object(["action": .object(["op": .string("mail.set_flag"), "native_key": .string("123"), "args": .object(["flagged": .bool(true)])])]))
    let proof = try await source.dispatch(prepared)
    try require(proof["observed"].bool && proof["native_key"].string == "123")
    try execute(path, "DELETE FROM messages WHERE ROWID=124")
    let sink = MailCommandSink()
    try await source.captureCommand(proof, into: sink)
    try require(await sink.messages.first?["native_key"].string == "123")
    let (_, after, complete) = try await source.snapshot()
    try require(complete && after.contains { $0["kind"].string == "absent" && $0["native_key"].string == "124" })
    let denied = MailSource(path: path, script: { _ in throw MailCommandFailure.permission })
    let failure = try await denied.dispatch(prepared)
    try require(!failure["observed"].bool && failure["disposition"].string == "indeterminate" && failure["reason"].string == MailCommandFailure.permission.localizedDescription)
}
func checkMailCommands() async throws {
    try await MailCommands.requireAutomationPermission { 0 }
    do { try await MailCommands.requireAutomationPermission { -1743 }; throw SourceFailure.corrupt }
    catch MailCommandFailure.permission {}
    let release = DispatchSemaphore(value: 0)
    let started = ContinuousClock.now
    do {
        try await MailCommands.requireAutomationPermission(after: 0.02) {
            _ = release.wait(timeout: .now() + 2)
            return 0
        }
        throw SourceFailure.corrupt
    } catch MailCommandFailure.timeout {}
    release.signal()
    try require(ContinuousClock.now - started < .seconds(1))
    try await MailCommands.requireAutomationPermission { 0 }

    let entered = DispatchSemaphore(value: 0)
    let resume = DispatchSemaphore(value: 0)
    let cancelledPermission = Task {
        try await MailCommands.requireAutomationPermission {
            entered.signal()
            _ = resume.wait(timeout: .now() + 2)
            return 0
        }
    }
    let didEnter = await withCheckedContinuation { continuation in
        DispatchQueue.global().async {
            continuation.resume(returning: entered.wait(timeout: .now() + 2) == .success)
        }
    }
    cancelledPermission.cancel()
    resume.signal()
    try require(didEnter)
    do { try await cancelledPermission.value; throw SourceFailure.corrupt }
    catch is CancellationError {}
    try require(MailCommandFailure.appleEvent(-1712) == .timeout)
    try require(MailCommandFailure.appleEvent(-1743) == .permission && MailCommandFailure.appleEvent(-1744) == .permission)
    try require(MailCommandFailure.appleEvent(-600) == .connectionLost && MailCommandFailure.appleEvent(-609) == .connectionLost)
    for op in MailCommands.operations {
        for observed in [true, false] {
            let fake = MailFake(observed ? "1" : "")
            let commands = MailCommands(script: { await fake.run($0) }, pollInterval: .milliseconds(1))
            let action: JSON = .object(["op": .string(op), "native_key": .string("123"), "args": .object(["to": .array([.string("self@example.invalid")]), "subject": .string("[ab-test synthetic]"), "body": .string("synthetic"), "account": .string("synthetic"), "mailbox": .string("Archive"), "flagged": .bool(true), "read": .bool(true)])])
            let prepared = try commands.prepare(.object(["action": action]))
            let result = try await commands.dispatch(prepared)
            try require(result["observed"].bool == observed)
            if observed { try require(result["native_key"].string == "123" && result["fingerprint"].string == prepared.fingerprint) }
            if !observed { try require(result["disposition"].string == "indeterminate" && result["reason"].string == MailCommandFailure.unconfirmed.localizedDescription) }
            let scripts = await fake.scripts()
            let creates = ["mail.create_draft", "mail.send"].contains(op)
            try require(scripts.count == (creates ? 2 : 1) + (observed ? (creates ? 2 : 1) : 40))
            let effect = scripts[creates ? 1 : 0]
            if creates { try require(effect.contains("visible:true")) }
            let expected = ["mail.create_draft": "save m", "mail.send": "send m", "mail.move": "move m to mailbox", "mail.delete": "move m to trash mailbox", "mail.mark_read": "set read status of m to true", "mail.set_flag": "set flagged status of m to true"]
            try require(effect.contains(expected[op]!))
            if op == "mail.create_draft" { try require(effect.contains("save m\nset visible of m to true\nclose m saving yes")) }
            try require(scripts.last!.contains("return"))
            if creates {
                try require(scripts.last!.contains(op == "mail.send" ? "sent mailbox" : "drafts mailbox"))
                try require(scripts.last!.contains("set candidateBody to (content of candidate) as string"))
                try require(!scripts.last!.contains("and content starts with"))
            }
        }
    }
    for expected in ["synthetic body", "\nsynthetic body"] {
        let fake = MailFake("1")
        let commands = MailCommands(script: { await fake.run($0) }, pollInterval: .milliseconds(1))
        let action: JSON = .object(["op": .string("mail.send"), "args": .object(["to": .array([.string("self@example.invalid")]), "subject": .string("synthetic subject"), "body": .string(expected)])])
        _ = try await commands.dispatch(commands.prepare(.object(["action": action])))
        let scripts = await fake.scripts()
        let query = "(messages of sent mailbox whose subject is \"synthetic subject\")"
        let readback = scripts[2].replacingOccurrences(of: query, with: "{fixtureMessage}")
        try require(!readback.contains("sent mailbox"))
        for (content, matches) in [(expected, true), ("\n" + expected + " \n", true), ("\r\n" + expected, true), ("prefix " + expected, false), ("", false), ("\n", false)] {
            let source = "script fixtureMessage\nproperty content : " + MailCommands.quote(content) + "\nend script\n" + readback
            let (value, succeeded) = await MainActor.run {
                var error: NSDictionary?
                let value = NSAppleScript(source: source)?.executeAndReturnError(&error).stringValue
                return (value, error == nil)
            }
            try require(succeeded && value == (matches ? "1" : "0"))
        }
    }
    let delayed = MailFake("true", lag: 2)
    let commands = MailCommands(script: { await delayed.run($0) }, pollInterval: .milliseconds(1))
    let prepared = try commands.prepare(.object(["action": .object(["op": .string("mail.set_flag"), "native_key": .string("123"), "args": .object(["flagged": .bool(true)])])]))
    try require(try await commands.dispatch(prepared)["observed"].bool)
    try require(await delayed.scripts().count == 4)
    let broken = MailCommands(script: { body in
        if body.contains("return (") { throw MailCommandFailure.timeout }
        return ""
    })
    do { _ = try await broken.dispatch(prepared); throw SourceFailure.corrupt }
    catch MailCommandFailure.timeout {}
    let unidentified = MailFake("1", identity: "")
    let sender = MailCommands(script: { await unidentified.run($0) })
    let send = try sender.prepare(.object(["action": .object(["op": .string("mail.send"), "args": .object(["to": .array([.string("self@example.invalid")]), "subject": .string("[ab-test] identity"), "body": .string("synthetic")])])]))
    do { _ = try await sender.dispatch(send); throw SourceFailure.corrupt }
    catch MailCommandFailure.unconfirmed {}
    try require(await unidentified.scripts().filter { $0.contains("send m") }.count == 1)
    #if LAB
    try await checkMailQualification()
    #endif
    try require(MailCommands.quote("\"\n\\") == "\"\\\"\\n\\\\\"")
}
