#if LAB
import Foundation
import MailSource
import SourceSupport

private actor QualificationMailFake {
    var calls: [String] = []
    var created = false
    var fail = false
    init(fail: Bool = false) { self.fail = fail }
    func run(_ body: String) throws -> String {
        calls.append(body)
        if body.hasPrefix("set matched to"), !body.contains("return count of matched") { return "123" }
        if body.contains("set remaining to 0") { created = false; return "0|0" }
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
    func scripts() -> [String] { calls }
}

func checkMailQualification() async throws {
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
        let proof = try await MailQualification(script: { try await fake.run($0) }).run(scenario: scenario, tag: "[ab-test] fixture")
        try require(!proof.object.isEmpty && proof.object.values.allSatisfy { $0 == .bool(true) })
        let scripts = await fake.scripts()
        try require(scripts.contains { $0.contains("set remaining to 0") } == (scenario != "MAIL-BODY-FIXTURE"))
        if scenario == "MAIL-BODY-FIXTURE" { try require(scripts.contains { $0.contains("set default message format to priorFormat") }) }
        if scenario != "MAIL-BODY-FIXTURE" {
            try require(scripts.contains { $0.contains("repeat 2 times") && $0.contains("by -1") && $0.contains("if (id of m) is not in trashed then delete m") && $0.contains("messages of trash mailbox whose subject is \"[ab-test] fixture\"") })
            try require(!scripts.contains { $0.contains("set deleted status") })
        }
        if scenario == "MAIL-ORGANIZE-REAL" { try require(scripts.contains { $0.contains("set flagged status of m to false") }) }
    }
    let failed = QualificationMailFake(fail: true)
    let runner = MailQualification(script: { try await failed.run($0) })
    let proof = try await runner.run(scenario: "MAIL-DRAFT-REAL", tag: "[ab-test] failed")
    try require(proof["fixture_and_commands"] == .bool(false) && proof["cleanup_passed"] == .bool(true))
    try require(proof["command_error"].string == MailCommandFailure.timeout.localizedDescription)
    let cleanupFailure = MailQualification(script: { _ in throw MailCommandFailure.permission })
    let cleanup = try await cleanupFailure.run(scenario: "MAIL-CLEANUP", tag: "[ab-test] failed")
    try require(cleanup["cleanup_passed"] == .bool(false) && cleanup["cleanup_error"].string == MailCommandFailure.permission.localizedDescription)
    for (raw, mailbox, outgoing) in [("1|0", false, true), ("0|1", true, false)] {
        let partial = MailQualification(script: { _ in raw })
        let proof = try await partial.run(scenario: "MAIL-CLEANUP", tag: "[ab-test] remaining")
        try require(!proof["cleanup_passed"].bool && proof["cleanup_mailbox_clear"].bool == mailbox && proof["cleanup_outgoing_clear"].bool == outgoing)
    }
    let count = await failed.scripts().count
    do { _ = try await runner.run(scenario: "MAIL-CLEANUP", tag: "foreign"); throw SourceFailure.corrupt } catch SourceFailure.invalid {}
    try require(await failed.scripts().count == count)
}
#endif
