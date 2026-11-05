import Foundation
@preconcurrency import MapKit
import CoreLocation
import SourceSupport

public actor MapsCommands: Source {
    public nonisolated let name = "maps"
    public init() {}
    public func status() async -> JSON? { .object([:]) }
    public func primitive(_ operation: String, args: JSON) async throws -> JSON {
        guard operation == "maps.show" else { throw SourceFailure.unsupported }
        return try await Self.show(args)
    }
    @MainActor private static func show(_ args: JSON) async throws -> JSON {
        let items: [MKMapItem]
        var options: [String: Any] = [:]
        if args["from"] != .null {
            items = [args["from"], args["to"]].map { MKMapItem(location: CLLocation(latitude: $0["lat"].number, longitude: $0["lon"].number), address: nil) }
            let modes = ["driving": MKLaunchOptionsDirectionsModeDriving, "walking": MKLaunchOptionsDirectionsModeWalking, "transit": MKLaunchOptionsDirectionsModeTransit, "cycling": MKLaunchOptionsDirectionsModeCycling]
            guard let mode = modes[args["mode"].string] else { throw SourceFailure.invalid }
            options[MKLaunchOptionsDirectionsModeKey] = mode
        } else {
            items = [MKMapItem(location: CLLocation(latitude: args["lat"].number, longitude: args["lon"].number), address: nil)]
        }
        guard await MKMapItem.openMaps(with: items, launchOptions: options) else { throw SourceFailure.unavailable }
        return .object([:])
    }
}
