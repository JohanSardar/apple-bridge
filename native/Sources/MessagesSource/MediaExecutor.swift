import Foundation

final class MediaExecutor: TaskExecutor {
    static let shared = MediaExecutor()
    private let queue = DispatchQueue(label: "applebridge.media", qos: .utility)

    func enqueue(_ job: consuming ExecutorJob) {
        let unowned = UnownedJob(job)
        queue.async { unowned.runSynchronously(on: self.asUnownedTaskExecutor()) }
    }
}
