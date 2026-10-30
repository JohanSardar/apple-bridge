import Foundation
import MailSource
import SourceSupport

actor MailFake {
    var calls: [String] = []
    let evidence: String
    let lag: Int
    var readbacks = 0
    init(_ evidence: String, lag: Int = 0) { self.evidence = evidence; self.lag = lag }
    func run(_ body: String) -> String {
        calls.append(body)
        if calls.count == 1 && body.hasPrefix("return") { return "0" }
        if body.hasPrefix("return") || body.contains("return (") {
            readbacks += 1
            return readbacks <= lag ? "" : evidence
        }
        return ""
    }
    func scripts() -> [String] { calls }
}
func checkMailCommands() async throws {
    for op in MailCommands.operations {
        for observed in [true, false] {
            let fake = MailFake(observed ? "1" : "")
            let commands = MailCommands(script: { await fake.run($0) })
            let action: JSON = .object(["op": .string(op), "native_key": .string("123"), "args": .object(["to": .array([.string("self@example.invalid")]), "subject": .string("[ab-test synthetic]"), "body": .string("synthetic"), "account": .string("synthetic"), "mailbox": .string("Archive"), "flagged": .bool(true), "read": .bool(true)])])
            let prepared = try commands.prepare(.object(["action": action]))
            let result = try await commands.dispatch(prepared)
            try require(result["observed"].bool == observed)
            if !observed { try require(result["disposition"].string == "indeterminate") }
            let scripts = await fake.scripts()
            let creates = ["mail.create_draft", "mail.send"].contains(op)
            try require(scripts.count == (creates ? 2 : 1) + (observed ? 1 : 40))
            let effect = scripts[creates ? 1 : 0]
            let expected = ["mail.create_draft": "save m", "mail.send": "send m", "mail.move": "move m to mailbox", "mail.delete": "move m to trash mailbox", "mail.mark_read": "set read status of m to true", "mail.set_flag": "set flagged status of m to true"]
            try require(effect.contains(expected[op]!))
            try require(scripts.last!.contains("return"))
            if creates { try require(scripts.last!.contains(op == "mail.send" ? "sent mailbox" : "drafts mailbox")) }
        }
    }
    let delayed = MailFake("true", lag: 2)
    let commands = MailCommands(script: { await delayed.run($0) })
    let prepared = try commands.prepare(.object(["action": .object(["op": .string("mail.set_flag"), "native_key": .string("123"), "args": .object(["flagged": .bool(true)])])]))
    try require(try await commands.dispatch(prepared)["observed"].bool)
    try require(await delayed.scripts().count == 4)
    #if LAB
    try await checkMailQualification()
    #endif
    try require(MailCommands.quote("\"\n\\") == "\"\\\"\\n\\\\\"")
}
