import Foundation
import CryptoKit

@main
struct ReleaseTools {
    static func main() {
        do { try execute() } catch {
            FileHandle.standardError.write(Data("Release verification failed: \(error.localizedDescription)\n".utf8))
            exit(1)
        }
    }
    static func execute() throws {
        let args = Array(CommandLine.arguments.dropFirst())
        if args.count == 2 && args[0] == "public-key" {
            let encoded = try String(contentsOfFile: args[1], encoding: .utf8).trimmingCharacters(in: .whitespacesAndNewlines)
            guard let seed = Data(base64Encoded: encoded), seed.count == 32 else { throw ReleaseError.invalidSignature }
            let key = try Curve25519.Signing.PrivateKey(rawRepresentation: seed)
            print(key.publicKey.rawRepresentation.base64EncodedString())
        } else if args.count == 4 && args[0] == "verify" {
            let items = try ReleaseFeed.parse(Data(contentsOf: URL(fileURLWithPath: args[1])))
            guard items.count == 1, let key = Data(base64Encoded: args[3]) else { throw ReleaseError.invalidFeed }
            try ReleaseSignature.verify(Data(contentsOf: URL(fileURLWithPath: args[2])), item: items[0], publicKey: key)
            print("R1-ARTIFACT-SIGNATURE passed=true")
        } else {
            throw ReleaseError.invalidFeed
        }
    }
}
