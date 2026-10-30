import ActivityKit

struct AgentTaskAttributes: ActivityAttributes {
    struct ContentState: Codable, Hashable {
        var status: String
        var progress: Double
    }
    let taskID: String
    let title: String
}
