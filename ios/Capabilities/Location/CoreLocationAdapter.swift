import CoreLocation
import Foundation

@MainActor
final class CoreLocationAdapter: NSObject, LocationAdapter, @preconcurrency CLLocationManagerDelegate {
    private let manager = CLLocationManager()
    private let significantManager = CLLocationManager()
    private var registrations: [String: CheckedContinuation<Void, Error>] = [:]
    private var registrationTimeouts: [String: Task<Void, Never>] = [:]
    var onEvent: ((String, String?, LocationFix?) -> Void)?
    private var pending: CheckedContinuation<LocationFix, Error>?
    private var timeout: Task<Void, Never>?
    private var significantEnabled = false

    override init() {
        super.init()
        manager.delegate = self
        significantManager.delegate = self
        manager.desiredAccuracy = kCLLocationAccuracyBest
    }

    func requestPermission() {
        manager.requestAlwaysAuthorization()
    }

    private func authorize(background: Bool) throws {
        guard CLLocationManager.locationServicesEnabled(), manager.authorizationStatus == .authorizedAlways || (!background && manager.authorizationStatus == .authorizedWhenInUse) else { throw LocationFailure.permission }
        if !background && manager.accuracyAuthorization != .fullAccuracy { throw LocationFailure.permission }
    }

    func current() async throws -> LocationFix {
        try authorize(background: false)
        guard pending == nil else { throw LocationFailure.busy }
        return try await withCheckedThrowingContinuation { continuation in
            pending = continuation
            manager.requestLocation()
            timeout = Task { [weak self] in
                try? await Task.sleep(for: .seconds(20))
                guard !Task.isCancelled else { return }
                self?.finish(.failure(LocationFailure.unavailable))
            }
        }
    }

    private func finish(_ result: Result<LocationFix, Error>) {
        let continuation = pending
        pending = nil
        timeout?.cancel()
        timeout = nil
        continuation?.resume(with: result)
    }

    func monitor(_ fence: NamedGeofence) async throws {
        try authorize(background: true)
        guard CLLocationManager.isMonitoringAvailable(for: CLCircularRegion.self), fence.radius <= manager.maximumRegionMonitoringDistance else { throw LocationFailure.unavailable }
        guard registrations[fence.name] == nil else { throw LocationFailure.busy }
        remove(fence.name)
        let region = CLCircularRegion(center: CLLocationCoordinate2D(latitude: fence.latitude, longitude: fence.longitude), radius: fence.radius, identifier: "ab.location." + fence.name)
        region.notifyOnEntry = true
        region.notifyOnExit = true
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            registrations[fence.name] = continuation
            manager.startMonitoring(for: region)
            registrationTimeouts[fence.name] = Task { [weak self] in
                try? await Task.sleep(for: .seconds(20))
                guard !Task.isCancelled, let self else { return }
                self.manager.stopMonitoring(for: region)
                self.finishMonitoring(fence.name, .failure(LocationFailure.unavailable))
            }
        }
    }

    private func finishMonitoring(_ name: String, _ result: Result<Void, Error>) {
        let continuation = registrations.removeValue(forKey: name)
        registrationTimeouts.removeValue(forKey: name)?.cancel()
        continuation?.resume(with: result)
    }

    func isMonitoring(_ name: String) -> Bool {
        manager.monitoredRegions.contains { $0.identifier == "ab.location." + name }
    }

    func remove(_ name: String) {
        finishMonitoring(name, .failure(LocationFailure.unavailable))
        for region in manager.monitoredRegions where region.identifier == "ab.location." + name { manager.stopMonitoring(for: region) }
    }

    func locationManager(_ manager: CLLocationManager, didStartMonitoringFor region: CLRegion) {
        guard region.identifier.hasPrefix("ab.location.") else { return }
        finishMonitoring(String(region.identifier.dropFirst("ab.location.".count)), .success(()))
    }

    func locationManager(_ manager: CLLocationManager, monitoringDidFailFor region: CLRegion?, withError error: Error) {
        if let region, region.identifier.hasPrefix("ab.location.") {
            finishMonitoring(String(region.identifier.dropFirst("ab.location.".count)), .failure(LocationFailure.unavailable))
        } else {
            for name in Array(registrations.keys) { finishMonitoring(name, .failure(LocationFailure.unavailable)) }
        }
    }

    func significant(_ enabled: Bool) throws {
        if enabled {
            try authorize(background: true)
            guard CLLocationManager.significantLocationChangeMonitoringAvailable() else { throw LocationFailure.unavailable }
            significantManager.startMonitoringSignificantLocationChanges()
        } else { significantManager.stopMonitoringSignificantLocationChanges() }
        significantEnabled = enabled
    }

    func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        guard let location = locations.last, location.horizontalAccuracy >= 0, manager === significantManager || abs(location.timestamp.timeIntervalSinceNow) <= 30 else { return }
        let fix = LocationFix(latitude: location.coordinate.latitude, longitude: location.coordinate.longitude, accuracy: location.horizontalAccuracy, timestamp: location.timestamp)
        if manager === self.manager && pending != nil { finish(.success(fix)) }
        if manager === significantManager && significantEnabled && UserDefaults.standard.bool(forKey: "capability.location.enabled") { onEvent?("significant", nil, fix) }
    }

    func locationManager(_ manager: CLLocationManager, didFailWithError error: Error) {
        if manager === self.manager { finish(.failure(LocationFailure.unavailable)) }
    }

    func locationManager(_ manager: CLLocationManager, didEnterRegion region: CLRegion) { event("arrive", region) }
    func locationManager(_ manager: CLLocationManager, didExitRegion region: CLRegion) { event("leave", region) }

    private func event(_ kind: String, _ region: CLRegion) {
        guard UserDefaults.standard.bool(forKey: "capability.location.enabled"), region.identifier.hasPrefix("ab.location.") else { return }
        onEvent?(kind, String(region.identifier.dropFirst("ab.location.".count)), nil)
    }

    func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        if manager.authorizationStatus == .denied || manager.authorizationStatus == .restricted { finish(.failure(LocationFailure.permission)) }
    }
}
