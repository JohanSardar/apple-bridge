#if LAB
import Foundation
import MailSource
import SourceSupport

private actor QualificationMailFake {
    var calls: [String] = []
    var created = false
    var fail = false
    var stalled = false
    init(fail: Bool = false, stalled: Bool = false) { self.fail = fail; self.stalled = stalled }
    func run(_ body: String) throws -> String {
        calls.append(body)
        if body == "return name" {
            if stalled { throw MailCommandFailure.timeout }
            return "Mail"
        }
        if body.hasPrefix("set matched to"), !body.contains("return count of matched") { return "123" }
        if body.contains("set chosen to selected messages") { return created ? "123" : "clear" }
        if body.hasPrefix("return count (outgoing") { return "0" }
        if body.contains("extract address from sender") { return "self@example.invalid" }
        if body.contains("set found to") { return created ? "123" : "" }
        if body.hasPrefix("return count") || body.contains("return count of matched") { return created ? "1" : "0" }
        if body.contains("make new outgoing message") {
            if fail { throw MailCommandFailure.timeout }
            created = true
        }
        if body.contains("return (flagged status") { return "true" }
        return ""
    }
    func remove() { created = false }
    func restart() { stalled = false }
    func scripts() -> [String] { calls }
}

func checkMailQualification() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let path = directory.appendingPathComponent("Envelope Index").path
    try execute(path, "CREATE TABLE subjects(subject TEXT); CREATE TABLE mailboxes(url TEXT); CREATE TABLE messages(subject INTEGER,mailbox INTEGER,deleted INTEGER); INSERT INTO subjects VALUES('[ab-test] fixture'),('[ab-test] other'); INSERT INTO mailboxes VALUES('imap://synthetic/INBOX'),('imap://synthetic/Trash'),('imap://synthetic/Drafts'); INSERT INTO messages VALUES(1,1,0),(2,1,0),(1,2,0),(1,1,1),(1,3,0);")
    try require(try MailFixtureCleanup.fixtureIDs(tag: "[ab-test] fixture", path: path) == ["1", "5"])
    try require(try MailFixtureCleanup.fixtureIDs(tag: "[ab-test] absent", path: path).isEmpty)

    let fired = DispatchSemaphore(value: 0)
    await MainActor.run { _ = MailQualification.deadline(after: 0.01) { _ = fired.signal() } }
    let firedResult = await withCheckedContinuation { continuation in
        DispatchQueue.global().async {
            continuation.resume(returning: fired.wait(timeout: .now() + 2) == .success)
        }
    }
    try require(firedResult)
    let cancelled = DispatchSemaphore(value: 0)
    await MainActor.run {
        let deadline = MailQualification.deadline(after: 0.01) { _ = cancelled.signal() }
        deadline.cancel()
    }
    let cancelledResult = await withCheckedContinuation { continuation in
        DispatchQueue.global().async {
            continuation.resume(returning: cancelled.wait(timeout: .now() + 0.05) == .timedOut)
        }
    }
    try require(cancelledResult)
    for scenario in ["MAIL-SEND-SELF-REAL", "MAIL-DRAFT-REAL", "MAIL-ORGANIZE-REAL", "MAIL-BODY-FIXTURE"] {
        let fake = QualificationMailFake()
        let proof = try await MailQualification(script: { try await fake.run($0) }, fixtureIDs: { _ in ["123"] }, removeSelected: { await fake.remove() }).run(scenario: scenario, tag: "[ab-test] fixture")
        try require(!proof.object.isEmpty && proof.object.values.allSatisfy { $0 == .bool(true) })
        let scripts = await fake.scripts()
        try require(scripts.contains { $0.contains("set chosen to selected messages") } == (scenario != "MAIL-BODY-FIXTURE"))
        if scenario == "MAIL-BODY-FIXTURE" { try require(scripts.contains { $0.contains("set default message format to priorFormat") }) }
        if scenario != "MAIL-BODY-FIXTURE" {
            try require(scripts.contains { $0.contains("set visible of m to true") && $0.contains("close m saving no") })
            try require(scripts.contains { $0.contains("{drafts mailbox, inbox, sent mailbox}") && $0.contains("if subject of item 1 of chosen is") && $0.contains("messages of v whose id is") })
            try require(!scripts.contains { $0.contains("delete m") || $0.contains("move m") || $0.contains("set deleted status") })
        }
        try require(scripts.contains { $0.contains("extract address from sender") && $0.contains("visible:true") && $0.contains("close m saving no") })
        if scenario == "MAIL-ORGANIZE-REAL" { try require(scripts.contains { $0.contains("set flagged status of m to false") }) }
    }
    let stalled = QualificationMailFake(stalled: true)
    let recovered = try await MailQualification(script: { try await stalled.run($0) }, fixtureIDs: { _ in ["123"] }, removeSelected: { await stalled.remove() }, restart: { await stalled.restart() }).run(scenario: "MAIL-DRAFT-REAL", tag: "[ab-test] recovered")
    try require(recovered["unresponsive_mail_recovered"].bool && recovered["draft_created"].bool && recovered["cleanup_passed"].bool)
    try require(await stalled.scripts().filter { $0.contains("save m") }.count == 1)
    let failed = QualificationMailFake(fail: true)
    let runner = MailQualification(script: { try await failed.run($0) }, fixtureIDs: { _ in ["123"] }, removeSelected: { await failed.remove() })
    let proof = try await runner.run(scenario: "MAIL-DRAFT-REAL", tag: "[ab-test] failed")
    try require(proof["fixture_and_commands"] == .bool(false) && proof["cleanup_passed"] == .bool(true))
    try require(proof["command_error"].string == MailCommandFailure.timeout.localizedDescription)
    let cleanupFailure = MailQualification(script: { _ in throw MailCommandFailure.permission })
    let cleanup = try await cleanupFailure.run(scenario: "MAIL-CLEANUP", tag: "[ab-test] failed")
    try require(cleanup["cleanup_passed"] == .bool(false) && cleanup["cleanup_error"].string.contains("close the tagged compose window"))
    for (raw, mailbox, outgoing) in [("1|0", false, true), ("0|1", true, false)] {
        let proof = try MailQualification.cleanupResult(raw)
        try require(!proof["cleanup_passed"]!.bool && proof["cleanup_mailbox_clear"]!.bool == mailbox && proof["cleanup_outgoing_clear"]!.bool == outgoing)
    }
    let unsafe = MailQualification(script: { body in body.contains("set chosen") ? "wrong" : "" }, fixtureIDs: { _ in ["123"] }, removeSelected: { throw SourceFailure.corrupt })
    let refused = try await unsafe.run(scenario: "MAIL-CLEANUP", tag: "[ab-test] refused")
    try require(!refused["cleanup_passed"].bool && refused["cleanup_error"].string == MailCommandFailure.unconfirmed.localizedDescription)
    for (stage, step) in [(6, "empty viewer"), (7, "only one test message"), (8, "test message identity"), (9, "displayed mailbox"), (10, "indexed test message")] {
        let selectionFailure = MailQualification(script: { body in body.contains("set chosen to selected messages") ? "failure|\(stage)|-2700" : "" }, fixtureIDs: { _ in ["123"] }, removeSelected: { throw SourceFailure.corrupt })
        let proof = try await selectionFailure.run(scenario: "MAIL-CLEANUP", tag: "[ab-test] selection")
        try require(!proof["cleanup_passed"].bool && proof["cleanup_error"].string.contains(step) && proof["cleanup_error_number"] == .number(-2700))
    }
    let count = await failed.scripts().count
    do { _ = try await runner.run(scenario: "MAIL-CLEANUP", tag: "foreign"); throw SourceFailure.corrupt } catch SourceFailure.invalid {}
    try require(await failed.scripts().count == count)
}
#endif
