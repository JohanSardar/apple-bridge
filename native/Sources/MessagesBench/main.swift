import Foundation
import Darwin
import CryptoKit
import MessagesSource
import SourceSupport

actor CounterSink: ObservationSink {
    nonisolated let completed: AsyncStream<Bool>
    private let continuation: AsyncStream<Bool>.Continuation
    private var counts: [String: Int] = [:]
    private var messageChats: [String: String] = [:]
    init() {
        let pair = AsyncStream<Bool>.makeStream()
        completed = pair.stream
        continuation = pair.continuation
    }
    func sourceState(source: String, state: String, coverage: JSON, capabilities: [String], reason: String) async throws {
        if state != "ready" { continuation.yield(false); continuation.finish() }
    }
    func emit(source: String, generation: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {
        for item in observations where item["kind"].string == "absent" && item["collection"].string == "messages.message" { messageChats.removeValue(forKey: item["native_key"].string) }
        for item in observations where item["kind"].string == "upsert" {
            guard try JSONEncoder().encode(item).count < 240000 else { throw SourceFailure.invalid }
            counts[item["collection"].string, default: 0] += 1
            if item["collection"].string == "messages.message" {
                messageChats[item["native_key"].string] = item["data"]["chat_id"]["$ref"]["native_key"].string
            }
        }
        if completionEvidence == "complete" { continuation.yield(true); continuation.finish() }
    }
    func saveCounts(_ url: URL) throws {
        var result: [String: Int] = [:]
        for chat in messageChats.values {
            let key = SHA256.hash(data: Data(chat.utf8)).map { String(format: "%02x", $0) }.joined()
            result[key, default: 0] += 1
        }
        try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys]).write(to: url)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
    }
    func order() -> Int {
        let count = messageChats.count
        return count == 0 ? 0 : Int(log10(Double(count)))
    }
}

@main
struct MessagesBench {
    static func main() async throws {
        let timeout = Task {
            try await Task.sleep(for: .seconds(600))
            print("{\"state\":\"benchmark_timeout\"}")
            exit(2)
        }
        defer { timeout.cancel() }
        let source = Messages()
        let readiness = await source.open()
        guard readiness.state == "ready" else { print("{\"state\":\"unavailable\"}"); exit(1) }
        let sink = CounterSink()
        let start = ContinuousClock.now
        let task = Task { try await source.capture(.null, into: sink) }
        var complete = false
        for await result in sink.completed { complete = result; break }
        guard complete else { task.cancel(); print("{\"state\":\"capture_failed\"}"); exit(1) }
        let elapsed = start.duration(to: .now)
        task.cancel()
        _ = try? await task.value
        var usage = rusage()
        getrusage(RUSAGE_SELF, &usage)
        let seconds = Double(elapsed.components.seconds) + Double(elapsed.components.attoseconds) / 1e18
        if let index = CommandLine.arguments.firstIndex(of: "--counts-file"), index + 1 < CommandLine.arguments.count {
            try await sink.saveCounts(URL(fileURLWithPath: CommandLine.arguments[index + 1]))
        }
        let report: [String: Any] = ["stage": "native_capture_to_acknowledging_sink", "observations_fit_ipc": true, "elapsed_seconds": seconds, "peak_rss_bytes": usage.ru_maxrss, "message_order_of_magnitude": await sink.order()]
        let data = try JSONSerialization.data(withJSONObject: report, options: [.sortedKeys])
        print(String(decoding: data, as: UTF8.self))
    }
}
