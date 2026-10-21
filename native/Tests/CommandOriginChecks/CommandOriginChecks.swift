import Foundation
import SourceSupport

@main
struct CommandOriginChecks {
    static func main() throws {
        func frame(_ principal: String) -> JSON {
            .object(["origin": .string("interactive"), "principal": .string(principal)])
        }
        let accepted = ["device:d", "agent:a", "replica:r/device:d", "replica:r/agent:a"]
        let rejected = ["other:r/agent:a", "replica:/agent:a", "replica:r/agent:", "replica:r/agent:a/extra", "replica:r/agent:a/device:d", "replica:r/device:", "other:r/device:d", "replica:r/agent:a\n", "replica:r/agent:a\r"]
        for principal in accepted {
            guard try CommandOrigin(frame(principal)) == .interactive else {
                throw NSError(domain: "CommandOriginChecks", code: 1)
            }
        }
        for principal in rejected {
            guard (try? CommandOrigin(frame(principal))) == nil else {
                throw NSError(domain: "CommandOriginChecks", code: 2)
            }
        }
        print("CommandOriginChecks: 13/13 passed")
    }
}
