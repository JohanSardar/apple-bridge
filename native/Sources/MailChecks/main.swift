import Foundation
import MailSource
import SourceSupport

func require(_ value: Bool, file: StaticString = #filePath, line: UInt = #line) throws {
    if !value {
        FileHandle.standardError.write(Data("Mail check failed at \(file):\(line)\n".utf8))
        throw SourceFailure.corrupt
    }
}

@main
struct Main {
    static func main() async throws {
        try await checkMailCommands()
        print("{}")
    }
}
