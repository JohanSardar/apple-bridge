import Foundation
@preconcurrency import CoreLocation

@MainActor
final class LocationPushService: NSObject, @preconcurrency CLLocationPushServiceExtension, @preconcurrency CLLocationManagerDelegate {
    private var manager: CLLocationManager?
    private var completion: (() -> Void)?

    func didReceiveLocationPushPayload(_ payload: [String: Any], completion: @escaping () -> Void) {
        finish()
        self.completion = completion
        guard Bundle.main.object(forInfoDictionaryKey: "ABApproved_locationPush") as? Bool == true, LocationPushState.enabled else { finish(); return }
        let manager = CLLocationManager()
        guard manager.authorizationStatus == .authorizedAlways else { finish(); return }
        manager.delegate = self
        manager.desiredAccuracy = kCLLocationAccuracyHundredMeters
        self.manager = manager
        manager.requestLocation()
    }

    func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        guard manager === self.manager else { return }
        defer { finish() }
        guard LocationPushState.enabled, let location = locations.last, location.horizontalAccuracy >= 0, let url = LocationPushState.snapshotURL else { return }
        let snapshot = LocationPushSnapshot(latitude: location.coordinate.latitude, longitude: location.coordinate.longitude, accuracy: location.horizontalAccuracy, date: location.timestamp)
        if let data = try? JSONEncoder().encode(snapshot) {
            try? data.write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        }
    }

    func locationManager(_ manager: CLLocationManager, didFailWithError error: Error) {
        if manager === self.manager { finish() }
    }
    func serviceExtensionWillTerminate() { finish() }

    private func finish() {
        manager?.stopUpdatingLocation()
        manager = nil
        let callback = completion
        completion = nil
        callback?()
    }
}
