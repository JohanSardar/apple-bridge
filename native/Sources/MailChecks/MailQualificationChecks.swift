#if LAB
import Foundation
import MailSource
import SourceSupport

private actor QualificationMailFake {
    var calls: [String] = []
    var created = false
    var fail = false
    var stalled = false
    var selectionMismatch = false
    var outgoing = false
    let outgoingAfterDelete: Bool
    let retainsClosedBackend: Bool
    var staleIndexReads = 0
    var visibleOutgoing = false
    init(fail: Bool = false, stalled: Bool = false, selectionMismatch: Bool = false, outgoingAfterDelete: Bool = false, retainsClosedBackend: Bool = false) { self.fail = fail; self.stalled = stalled; self.selectionMismatch = selectionMismatch; self.outgoingAfterDelete = outgoingAfterDelete; self.retainsClosedBackend = retainsClosedBackend }
    func run(_ body: String) throws -> String {
        calls.append(body)
        if body == "return name" {
            if stalled { throw MailCommandFailure.timeout }
            return "Mail"
        }
        if body.hasPrefix("set matched to"), !body.contains("return count of matched") { return "123" }
        if body.contains("set cleanupStage") { return created ? "select|fixture@example.invalid" : "clear" }
        if body.contains("name of front window contains") { return created && !selectionMismatch ? "opened" : "" }
        if body.contains("set ownedIDs") { if !retainsClosedBackend { outgoing = false }; visibleOutgoing = false; return "" }
        if body.hasPrefix("return count (outgoing") { return (body.contains("visible is true") ? visibleOutgoing : outgoing) ? "1" : "0" }
        if body.hasPrefix("return count (windows") { return visibleOutgoing ? "1" : "0" }
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
    func remove() { created = false; outgoing = outgoingAfterDelete; visibleOutgoing = outgoingAfterDelete }
    func ids() -> [String] {
        if !created, staleIndexReads > 0 { staleIndexReads -= 1; return ["123"] }
        return created ? ["123"] : []
    }
    func lagIndex() { staleIndexReads = 2 }
    func select() { selectionMismatch = false }
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

    try execute(path, "INSERT INTO subjects VALUES('[ab-test] orphan'); INSERT INTO messages VALUES(3,999,0);")
    do { _ = try MailFixtureCleanup.fixtureIDs(tag: "[ab-test] orphan", path: path); throw MailCommandFailure.rejected }
    catch SourceFailure.corrupt {}
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
        let proof = try await MailQualification(script: { try await fake.run($0) }, fixtureIDs: { _ in await fake.ids() }, selectDisplayed: { _ in await fake.select() }, removeSelected: { await fake.remove() }).run(scenario: scenario, tag: "[ab-test] fixture")
        try require(!proof.object.isEmpty && proof.object.values.allSatisfy { $0 == .bool(true) })
        let scripts = await fake.scripts()
        for body in scripts {
            let valid = await MainActor.run {
                var error: NSDictionary?
                return NSAppleScript(source: "with timeout of 15 seconds\ntell application id \"com.apple.mail\"\n" + body + "\nend tell\nend timeout")?.compileAndReturnError(&error) == true
            }
            try require(valid)
        }
        try require(scripts.contains { $0.contains("name of front window contains") } == (scenario != "MAIL-BODY-FIXTURE"))
        if scenario == "MAIL-BODY-FIXTURE" { try require(scripts.contains { $0.contains("set default message format to priorFormat") }) }
        if scenario != "MAIL-BODY-FIXTURE" {
            try require(scripts.contains { $0.contains("windows whose name contains") && $0.contains("close m saving no") && !$0.contains("set visible of m to true") })
            try require(scripts.contains { $0.contains("{drafts mailbox, inbox, sent mailbox}") && $0.contains("set headerID to message id of m") })
            try require(scripts.contains { $0.contains("if own is in ownRecipients") })
            try require(!scripts.contains { $0.contains("delete m") || $0.contains("move m") || $0.contains("set deleted status") || $0.contains("set selected messages") })
        }
        try require(scripts.contains { $0.contains("extract address from sender") && $0.contains("visible:true") && $0.contains("close m saving no") })
        if scenario == "MAIL-ORGANIZE-REAL" { try require(scripts.contains { $0.contains("set flagged status of m to false") }) }
    }
    let delayedIndex = QualificationMailFake()
    await delayedIndex.lagIndex()
    let delayedProof = try await MailQualification(script: { try await delayedIndex.run($0) }, fixtureIDs: { _ in await delayedIndex.ids() }, selectDisplayed: { header in
        try require(header == "fixture@example.invalid")
        await delayedIndex.select()
    }, removeSelected: { await delayedIndex.remove() }).run(scenario: "MAIL-ORGANIZE-REAL", tag: "[ab-test] delayed")
    try require(delayedProof["cleanup_passed"].bool)
    let stalled = QualificationMailFake(stalled: true)
    let recovered = try await MailQualification(script: { try await stalled.run($0) }, fixtureIDs: { _ in await stalled.ids() }, selectDisplayed: { _ in await stalled.select() }, removeSelected: { await stalled.remove() }, restart: { await stalled.restart() }).run(scenario: "MAIL-DRAFT-REAL", tag: "[ab-test] recovered")
    try require(recovered["unresponsive_mail_recovered"].bool && recovered["draft_created"].bool && recovered["cleanup_passed"].bool)
    try require(await stalled.scripts().filter { $0.contains("save m") }.count == 1)
    let failed = QualificationMailFake(fail: true)
    let runner = MailQualification(script: { try await failed.run($0) }, fixtureIDs: { _ in await failed.ids() }, selectDisplayed: { _ in await failed.select() }, removeSelected: { await failed.remove() })
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
    for (stage, step) in [(1, "find the tagged"), (2, "read the tagged"), (3, "show the tagged"), (4, "choose the tagged"), (10, "indexed test message")] {
        let selectionFailure = MailQualification(script: { body in body.contains("set cleanupStage") ? "failure|\(stage)|-2700" : "" }, fixtureIDs: { _ in ["123"] }, removeSelected: { throw SourceFailure.corrupt })
        let proof = try await selectionFailure.run(scenario: "MAIL-CLEANUP", tag: "[ab-test] selection")
        try require(!proof["cleanup_passed"].bool && proof["cleanup_error"].string.contains(step) && proof["cleanup_error_number"] == .number(-2700))
    }
    let mismatch = QualificationMailFake(selectionMismatch: true)
    let selected = try await MailQualification(script: { try await mismatch.run($0) }, fixtureIDs: { _ in await mismatch.ids() }, selectDisplayed: { _ in await mismatch.select() }, removeSelected: { await mismatch.remove() }).run(scenario: "MAIL-DRAFT-REAL", tag: "[ab-test] selection")
    try require(selected["draft_created"].bool && selected["cleanup_passed"].bool)
    let unsafeSelection = MailQualification(script: { body in body.contains("set cleanupStage") ? "select|123" : "" }, fixtureIDs: { _ in ["123"] }, selectDisplayed: { _ in }, removeSelected: { throw SourceFailure.corrupt })
    let unconfirmedSelection = try await unsafeSelection.run(scenario: "MAIL-CLEANUP", tag: "[ab-test] selection")
    try require(!unconfirmedSelection["cleanup_passed"].bool && unconfirmedSelection["cleanup_error"].string == MailCommandFailure.unconfirmed.localizedDescription)
    let reopened = QualificationMailFake(outgoingAfterDelete: true)
    let reopenedProof = try await MailQualification(script: { try await reopened.run($0) }, fixtureIDs: { _ in await reopened.ids() }, selectDisplayed: { _ in await reopened.select() }, removeSelected: { await reopened.remove() }).run(scenario: "MAIL-DRAFT-REAL", tag: "[ab-test] reopened")
    try require(reopenedProof["cleanup_passed"].bool && reopenedProof["cleanup_outgoing_clear"].bool)
    try require(await !reopened.outgoing)
    let cached = QualificationMailFake(outgoingAfterDelete: true, retainsClosedBackend: true)
    let cachedProof = try await MailQualification(script: { try await cached.run($0) }, fixtureIDs: { _ in await cached.ids() }, selectDisplayed: { _ in await cached.select() }, removeSelected: { await cached.remove() }).run(scenario: "MAIL-DRAFT-REAL", tag: "[ab-test] cached")
    try require(cachedProof["cleanup_passed"].bool && cachedProof["cleanup_windows_clear"].bool)
    let staleVisible = MailQualification(script: { body in
        if body.contains("set cleanupStage") { return "clear" }
        if body.contains("outgoing messages"), !body.contains("windows whose name contains") { throw MailCommandFailure.timeout }
        if body.hasPrefix("return count (windows") { return "0" }
        return ""
    }, fixtureIDs: { _ in [] }, removeSelected: { throw SourceFailure.corrupt })
    let staleVisibleProof = try await staleVisible.run(scenario: "MAIL-CLEANUP", tag: "[ab-test] cached")
    try require(staleVisibleProof["cleanup_passed"].bool && staleVisibleProof["cleanup_windows_clear"].bool)
    let openWindow = MailQualification(script: { body in
        if body.contains("set cleanupStage") { return "clear" }
        if body.hasPrefix("return count (windows") { return "1" }
        if body.hasPrefix("return count") { return "0" }
        return ""
    }, fixtureIDs: { _ in [] }, removeSelected: { throw SourceFailure.corrupt })
    let windowProof = try await openWindow.run(scenario: "MAIL-CLEANUP", tag: "[ab-test] window")
    try require(!windowProof["cleanup_passed"].bool && !windowProof["cleanup_outgoing_clear"].bool && !windowProof["cleanup_windows_clear"].bool)
    let staleSelection = MailQualification(script: { body in
        if body.contains("set cleanupStage") { return "select|123" }
        if body.contains("set chosen") { return "456" }
        return ""
    }, fixtureIDs: { _ in ["123"] }, selectDisplayed: { _ in }, removeSelected: { throw SourceFailure.corrupt })
    let stale = try await staleSelection.run(scenario: "MAIL-CLEANUP", tag: "[ab-test] selection")
    try require(!stale["cleanup_passed"].bool && stale["cleanup_error"].string == MailCommandFailure.unconfirmed.localizedDescription)
    let count = await failed.scripts().count
    do { _ = try await runner.run(scenario: "MAIL-CLEANUP", tag: "foreign"); throw SourceFailure.corrupt } catch SourceFailure.invalid {}
    try require(await failed.scripts().count == count)
}
#endif
