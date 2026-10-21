import Foundation
import SourceSupport

@MainActor
final class StoreMainQueueProbe {
    private var delays: [Double] = []

    func observe(_ sent: Double) { delays.append((ProcessInfo.processInfo.systemUptime - sent) * 1000) }

    var json: JSON {
        let sorted = delays.sorted()
        let p95 = sorted.isEmpty ? 0 : sorted[min(sorted.count - 1, Int(Double(sorted.count) * 0.95))]
        return .object(["samples": .number(Double(sorted.count)), "p95_ms": .number(p95), "max_ms": .number(sorted.last ?? 0), "under_16_ms": .bool(!sorted.isEmpty && (sorted.last ?? 0) < 16)])
    }
}

public enum RemindersStoreResponsiveness {
    @MainActor
    public static func run() async throws -> JSON {
        let probe = StoreMainQueueProbe()
        let queue = DispatchQueue(label: "dev.applebridge.reminders-store-heartbeat", qos: .userInteractive)
        let timer = DispatchSource.makeTimerSource(queue: queue)
        timer.schedule(deadline: .now(), repeating: .milliseconds(2))
        timer.setEventHandler { @Sendable in
            let sent = ProcessInfo.processInfo.systemUptime
            DispatchQueue.main.async { probe.observe(sent) }
        }
        timer.resume()
        defer { timer.cancel() }
        let elapsed = try await Task.detached {
            let paths = try RemindersStore.paths()
            let start = ProcessInfo.processInfo.systemUptime
            for path in paths {
                let session = try RemindersStoreSession(path: path)
                _ = try await session.read(full: true)
            }
            return (ProcessInfo.processInfo.systemUptime - start) * 1000
        }.value
        timer.cancel()
        try await Task.sleep(for: .milliseconds(10))
        return .object(["cold_full_elapsed_ms": .number(elapsed), "main_queue_delay": probe.json])
    }
}
