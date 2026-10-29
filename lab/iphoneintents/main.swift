import Foundation

@MainActor
enum CommandHandlers {
    static var all: [String: (Data) async throws -> Data] {
        ["iphone.ping": { data in
            let object = try JSONSerialization.jsonObject(with: data) as? [String: String]
            guard object?["device_id"] == "iphone:local" else { throw ProofError.failed }
            return Data("{\"pong\":true}".utf8)
        }]
    }
}

enum ProofError: Error { case failed }

@main
struct IntentDispatchProof {
    @MainActor
    static func main() async throws {
        guard try await CommandIntent.run("iphone.ping") == "{\"pong\":true}" else { throw ProofError.failed }
        guard try await CommandIntent.run("iphone.ping", arguments: "{\"device_id\":\"other\"}") == "{\"pong\":true}" else { throw ProofError.failed }
        do {
            _ = try await CommandIntent.run("iphone.disabled")
            throw ProofError.failed
        } catch CommandIntentError.unavailable {}
        do {
            _ = try await CommandIntent.run("iphone.ping", arguments: "[]")
            throw ProofError.failed
        } catch CommandIntentError.invalidArguments {}
        print("intent_dispatch_check: pass assertions=4")
    }
}
