import Foundation
import CoreLocation
import SourceSupport

public actor LocationStatus: Source {
    public nonisolated let name = "location"
    public init() {}
    public func open() async -> SourceReadiness { await Self.readiness() }
    @MainActor private static let authorization = LocationAuthorization()
    @MainActor private static func readiness() -> SourceReadiness {
        let status = authorization.status
        let reason: String
        switch status {
        case .notDetermined: reason = "not_determined"
        case .denied: reason = "denied"
        case .restricted: reason = "restricted"
        case .authorizedAlways: reason = ""
        default: reason = "unavailable"
        }
        return SourceReadiness(state: reason.isEmpty ? "ready" : "permission_missing", coverage: .object(["fields": .string("complete")]), reason: reason)
    }
    public func capture(_ resume: JSON, into sink: any ObservationSink) async throws { try await Task.sleep(for: .seconds(30)) }
    public func prepare(_ command: JSON) async throws -> Prepared { throw SourceFailure.unsupported }
    public func dispatch(_ prepared: Prepared) async throws -> JSON { throw SourceFailure.unsupported }
    public func resolve(_ command: String, evidence: JSON) async -> JSON { .null }
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
