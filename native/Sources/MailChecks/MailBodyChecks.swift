import Foundation
import MailSource
import SourceSupport

func checkMailBodies(_ root: URL) async throws {
    let fixtures = URL(fileURLWithPath: "lab/fixtures/mail")
    for name in ["1.emlx", "2.emlx", "3.partial.emlx", "4.emlx"] {
        try FileManager.default.copyItem(at: fixtures.appendingPathComponent(name), to: root.appendingPathComponent(name))
    }
    func read(_ id: String) throws -> JSON { try MailBody.read(.object(["message_id": .string(id)]), root: root) }
    let plain = try read("1"), alternative = try read("2"), partial = try read("3"), missing = try read("4"), offloaded = try read("5")
    try require(plain["plain_text"].string == "Hello fixture" && plain["state"].string == "downloaded")
    try require(alternative["plain_text"].string == "Alternative" && alternative["html"].string.contains("&lt;script&gt;") && !alternative["html"].string.contains("<script") && !alternative["html"].string.contains("<img"))
    let multipart = "Content-Type: multipart/alternative; boundary=fixture\n\n--fixture\nContent-Type: text/plain\n\n\n> [ab-test] fixture\n\n--fixture--\n"
    try (String(multipart.utf8.count) + "\n" + multipart).write(to: root.appendingPathComponent("9.emlx"), atomically: true, encoding: .utf8)
    let preserved = try read("9")
    try require(preserved["state"].string == "downloaded" && preserved["plain_text"].string == "\n> [ab-test] fixture\n")
    let paddedLength = String(multipart.utf8.count).padding(toLength: 10, withPad: " ", startingAt: 0)
    try (paddedLength + "\n" + multipart).write(to: root.appendingPathComponent("10.emlx"), atomically: true, encoding: .utf8)
    let padded = try read("10")
    try require(padded["state"].string == "downloaded" && padded["plain_text"] == preserved["plain_text"])
    try require(partial["state"].string == "partial" && missing["state"].string == "partial" && offloaded["state"].string == "offloaded")
    let htmlOnly = "Content-Type: text/html\n\n<p>Hello</p><script>bad()</script>"
    try (String(htmlOnly.utf8.count) + "\n" + htmlOnly).write(to: root.appendingPathComponent("8.emlx"), atomically: true, encoding: .utf8)
    let htmlBody = try read("8")
    try require(htmlBody["plain_text"].string == "Hello")
    let latin = Data([67, 97, 102, 233])
    let prefix = Data("Content-Type: text/plain; charset=iso-8859-1\n\n".utf8)
    let message = prefix + latin
    try (Data((String(message.count) + "\n").utf8) + message).write(to: root.appendingPathComponent("7.emlx"))
    let decodedLatin = try read("7")
    try require(decodedLatin["plain_text"].string == "Café")
    let large = "Content-Type: text/plain\n\n" + String(repeating: "a", count: 1100000)
    try (String(large.utf8.count) + "\n" + large).write(to: root.appendingPathComponent("6.emlx"), atomically: true, encoding: .utf8)
    let largeResult = try read("6")
    let largeBytes = try JSONEncoder().encode(largeResult)
    try require(largeBytes.count > 1 << 20)
    do { _ = try read("../1"); throw SourceFailure.corrupt } catch SourceFailure.invalid {}
}
