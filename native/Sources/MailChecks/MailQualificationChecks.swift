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
        if body.contains("set remaining to 0") { created = false; return "true" }
        if body.contains("extract address from sender") { return "self@example.invalid" }
        if body.contains("set found to") { return created ? "123" : "" }
        if body.hasPrefix("return count") { return created ? "1" : "0" }
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
    for scenario in ["MAIL-SEND-SELF-REAL", "MAIL-DRAFT-REAL", "MAIL-ORGANIZE-REAL", "MAIL-BODY-FIXTURE"] {
        let fake = QualificationMailFake()
        let proof = try await MailQualification(script: { try await fake.run($0) }).run(scenario: scenario, tag: "[ab-test] fixture")
        try require(!proof.object.isEmpty && proof.object.values.allSatisfy { $0 == .bool(true) })
        let scripts = await fake.scripts()
        try require(scripts.contains { $0.contains("set remaining to 0") } == (scenario != "MAIL-BODY-FIXTURE"))
        if scenario == "MAIL-BODY-FIXTURE" { try require(scripts.contains { $0.contains("set default message format to priorFormat") }) }
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
    let count = await failed.scripts().count
    do { _ = try await runner.run(scenario: "MAIL-CLEANUP", tag: "foreign"); throw SourceFailure.corrupt } catch SourceFailure.invalid {}
    try require(await failed.scripts().count == count)
}
#endif
