import Foundation
import CoreLocation
import SourceSupport

public actor LocationStatus: Source {
    public nonisolated let name = "location"
    public init() {}
    public func status() async -> JSON? { await Self.authorizationName() }
    @MainActor private static let authorization = LocationAuthorization()
    @MainActor private static func authorizationName() -> JSON {
        let name: String
        switch authorization.status {
        case .notDetermined: name = "not_determined"
        case .denied: name = "denied"
        case .restricted: name = "restricted"
        case .authorizedAlways: name = "authorized_always"
        default: name = "unavailable"
        }
        return .object(["authorization": .string(name)])
    }
}

@MainActor
private final class LocationAuthorization: NSObject, @preconcurrency CLLocationManagerDelegate {
    private let manager = CLLocationManager()
    private(set) var status: CLAuthorizationStatus
    override init() {
        status = manager.authorizationStatus
        super.init()
        manager.delegate = self
    }
    func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        status = manager.authorizationStatus
    }
}
