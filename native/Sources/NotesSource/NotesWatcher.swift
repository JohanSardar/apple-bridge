import Foundation
import CoreServices

final class NotesWatcher: @unchecked Sendable {
    let changes: AsyncStream<Bool>
    private let continuation: AsyncStream<Bool>.Continuation
    private let queue = DispatchQueue(label: "applebridge.notes.events")
    private var events: FSEventStreamRef?
    private var timer: (any DispatchSourceTimer)?
    init(path: String) {
        let pair = AsyncStream<Bool>.makeStream(bufferingPolicy: .bufferingNewest(1))
        changes = pair.stream; continuation = pair.continuation
        var context = FSEventStreamContext(version: 0, info: Unmanaged.passUnretained(self).toOpaque(), retain: nil, release: nil, copyDescription: nil)
        events = FSEventStreamCreate(nil, { _, info, _, _, _, _ in
            guard let info else { return }
            Unmanaged<NotesWatcher>.fromOpaque(info).takeUnretainedValue().continuation.yield(true)
        }, &context, [path] as CFArray, FSEventStreamEventId(kFSEventStreamEventIdSinceNow), 0.1, UInt32(kFSEventStreamCreateFlagFileEvents | kFSEventStreamCreateFlagNoDefer))
        if let events { FSEventStreamSetDispatchQueue(events, queue); _ = FSEventStreamStart(events) }
        let timer = DispatchSource.makeTimerSource(queue: queue)
        timer.schedule(deadline: .now() + 300, repeating: 300)
        timer.setEventHandler { [weak self] in self?.continuation.yield(true) }
        timer.resume(); self.timer = timer
        continuation.yield(true)
    }
    func close() {
        if let events { FSEventStreamStop(events); FSEventStreamInvalidate(events); queue.sync {}; FSEventStreamRelease(events) }
        events = nil; timer?.cancel(); timer = nil; continuation.finish()
    }
}
