import Foundation

public final class MediaExecutor: TaskExecutor {
    public static let shared = MediaExecutor()
    private let queue = DispatchQueue(label: "applebridge.media", qos: .utility)

    public func enqueue(_ job: consuming ExecutorJob) {
        let unowned = UnownedJob(job)
        queue.async { unowned.runSynchronously(on: self.asUnownedTaskExecutor()) }
    }
}
