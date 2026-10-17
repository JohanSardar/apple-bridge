import Foundation
import Decoders

@main
struct NotesDecoderChecks {
    static func main() throws {
        let root = CommandLine.arguments[1]
        for (name, rich) in [("plain", false), ("formatting", false), ("checklist", true), ("table", true), ("attachment", true)] {
            let bytes = try Data(contentsOf: URL(fileURLWithPath: root + "/" + name + ".gz"))
            let decoded = try NotesProto.decode(bytes)
            guard decoded.text == "[ab-test decoder] Title\nbody 🧪\n", decoded.rich == rich else { throw CheckFailure.mismatch }
            for length in 0..<bytes.count {
                guard (try? NotesProto.decode(bytes.prefix(length))) == nil else { throw CheckFailure.truncation }
            }
        }
        guard (try? NotesProto.decode(Data(contentsOf: URL(fileURLWithPath: root + "/malformed.gz")))) == nil else { throw CheckFailure.malformed }
        print("Notes decoder fixtures passed")
    }
    private enum CheckFailure: Error { case mismatch, truncation, malformed }
}
