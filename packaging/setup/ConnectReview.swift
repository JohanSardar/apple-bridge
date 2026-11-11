import Foundation

@main
struct ConnectReview {
    @MainActor
    static func main() async {
        let model = ConnectModel(home: URL(fileURLWithPath: "/tmp/ab-review-unused"), testing: false) { _, _ in
            Data(#"{"remote":{"routes":{"system":{"state":"off","sources":["calendar","messages"]}}}}"#.utf8)
        }
        await model.refresh()
        precondition(model.cloudSources == ["calendar", "messages"])
        model.cloudSources = ["calendar"]
        await model.refresh()
        guard model.cloudSources == ["calendar"] else {
            print("FAIL: refresh restored a source the owner unticked: \(model.cloudSources.sorted())")
            exit(1)
        }
    }
}
