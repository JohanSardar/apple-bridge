import Foundation
import AppKit
@preconcurrency import MapKit
import CoreLocation
import SourceSupport

public actor MapsCommands: Source {
    public nonisolated let name = "maps"
    public init() {}
    public func open() async -> SourceReadiness {
        SourceReadiness(state: "ready", coverage: .object(["fields": .string("complete")]), capabilities: ["maps.open"])
    }
    public func capture(_ resume: JSON, into sink: any ObservationSink) async throws { try await Task.sleep(for: .seconds(30)) }
    public func prepare(_ command: JSON) async throws -> Prepared {
        let action = command["action"]
        guard action["op"].string == "maps.open" else { throw SourceFailure.invalid }
        let args = action["args"]
        if args["from"] != .null || args["to"] != .null {
            guard Set(args.object.keys) == ["from", "to", "mode"], Self.validPoint(args["from"]), Self.validPoint(args["to"]), ["driving", "walking", "transit", "cycling"].contains(args["mode"].string) else { throw SourceFailure.invalid }
        } else {
            guard Self.validPoint(args) else { throw SourceFailure.invalid }
        }
        return Prepared(nativeState: .object(["route": .string("maps")]), fingerprint: command["request_hash"].string, action: action)
    }
    public func dispatch(_ prepared: Prepared) async throws -> JSON {
        let args = prepared.action["args"]
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
        for _ in 0..<40 {
            if NSWorkspace.shared.frontmostApplication?.bundleIdentifier == "com.apple.Maps" {
                return .object(["disposition": .string("locally_observed"), "frontmost_bundle": .string("com.apple.Maps")])
            }
            try await Task.sleep(for: .milliseconds(100))
        }
        return .object(["disposition": .string("indeterminate")])
    }
    private static func validPoint(_ args: JSON) -> Bool {
        Set(args.object.keys) == ["lat", "lon"] && args["lat"] == .number(args["lat"].number) && args["lon"] == .number(args["lon"].number) && abs(args["lat"].number) <= 90 && abs(args["lon"].number) <= 180
    }
    public func resolve(_ command: String, evidence: JSON) async -> JSON {
        guard evidence["frontmost_bundle"].string == "com.apple.Maps", !evidence["prepared"]["request_hash"].string.isEmpty, evidence["prepared"]["action"]["op"].string == "maps.open" else { return .null }
        var result = evidence.object
        result["observed"] = .bool(true)
        result["disposition"] = .string("locally_observed")
        return .object(result)
    }
}
