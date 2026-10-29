#if os(watchOS)
import Foundation
import CoreMotion

final class WatchFallDetection: NSObject, CMFallDetectionDelegate {
    private let lock = NSLock()
    private var manager: CMFallDetectionManager?
    private var lastDate: Date?
    private let receive: (Date, Int) -> Void

    init(receive: @escaping (Date, Int) -> Void) {
        self.receive = receive
    }

    func start(approved: Bool, enabled: Bool) throws {
        guard approved && enabled && CMFallDetectionManager.isAvailable else { throw CocoaError(.featureUnsupported) }
        try lock.withLock {
            guard manager == nil else { return }
            let manager = CMFallDetectionManager()
            guard manager.authorizationStatus == .authorized else { throw CocoaError(.fileReadNoPermission) }
            self.manager = manager
            manager.delegate = self
        }
    }

    func stop() {
        lock.withLock {
            manager?.delegate = nil
            manager = nil
        }
    }

    func fallDetectionManager(_ fallDetectionManager: CMFallDetectionManager, didDetect event: CMFallDetectionEvent, completionHandler handler: @escaping () -> Void) {
        defer { handler() }
        let deliver = lock.withLock {
            guard manager === fallDetectionManager, lastDate != event.date else { return false }
            lastDate = event.date
            return true
        }
        if deliver { receive(event.date, event.resolution.rawValue) }
    }
}
#endif
