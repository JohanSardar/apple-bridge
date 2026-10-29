import Foundation
import AppKit
import CryptoKit
@preconcurrency import MapKit
@preconcurrency import CoreLocation
import SourceSupport

public enum MapsQueries {
    @MainActor private static var snapshots: [String: (Date, Data, String)] = [:]
    @MainActor private static var active = 0

    @MainActor public static func readBlob(_ handle: String, offset: Int, length: Int) throws -> JSON {
        snapshots = snapshots.filter { $0.value.0 > Date() }
        guard let snapshot = snapshots[handle], offset >= 0, offset <= snapshot.1.count, length > 0 else { throw SourceFailure.unavailable }
        let data = snapshot.1
        let end = min(data.count, offset + length)
        if end == data.count { snapshots.removeValue(forKey: handle) }
        return .object(["t": .string("BlobChunk"), "handle": .string(handle), "offset": .number(Double(offset)), "bytes": .string(data.subdata(in: offset..<end).base64EncodedString()), "digest": .string(snapshot.2), "eof": .bool(end == data.count)])
    }

    @MainActor
    public static func snapshot(_ args: JSON) async throws -> QueryOutput {
        try Task.checkCancellation()
        guard active < 8 else { throw SourceFailure.unavailable }
        active += 1
        defer { active -= 1 }
        try fields(args, ["center", "span", "width", "height", "markers"])
        let center = try point(args["center"])
        let span = args["span"].number
        let width = args["width"].number
        let height = args["height"].number
        guard span > 0, span <= 180, abs(center.latitude) + span / 2 <= 90, width >= 128, width <= 1024, height >= 128, height <= 1024, width.rounded() == width, height.rounded() == height else { throw SourceFailure.invalid }
        guard args["markers"] == .null || (args["markers"] == .array(args["markers"].array) && args["markers"].array.count <= 50) else { throw SourceFailure.invalid }
        let markers = try args["markers"].array.map { try point($0) }
        snapshots = snapshots.filter { $0.value.0 > Date() }
        guard snapshots.count < 16 else { throw SourceFailure.unavailable }
        let options = MKMapSnapshotter.Options()
        options.region = MKCoordinateRegion(center: center, span: MKCoordinateSpan(latitudeDelta: span, longitudeDelta: span))
        options.size = CGSize(width: width, height: height)
        let snapshotter = MKMapSnapshotter(options: options)
        let cancellation = NativeCancellation { snapshotter.cancel() }
        let snapshot = try await withTaskCancellationHandler { try await snapshotter.start() } onCancel: { cancellation.cancel() }
        try Task.checkCancellation()
        guard let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(width), pixelsHigh: Int(height), bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0), let context = NSGraphicsContext(bitmapImageRep: bitmap) else { throw SourceFailure.unavailable }
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = context
        snapshot.image.draw(in: CGRect(x: 0, y: 0, width: width, height: height))
        NSGraphicsContext.saveGraphicsState()
        NSBezierPath(rect: CGRect(x: 0, y: 32, width: width, height: height - 32)).addClip()
        for marker in markers {
            let p = snapshot.point(for: marker)
            guard p.x.isFinite, p.y.isFinite else { continue }
            NSColor.systemRed.setFill()
            NSBezierPath(ovalIn: CGRect(x: p.x - 4, y: p.y - 4, width: 8, height: 8)).fill()
        }
        NSGraphicsContext.restoreGraphicsState()
        let attribution = "© Apple Maps" as NSString
        attribution.draw(at: CGPoint(x: 8, y: height - 20), withAttributes: [.font: NSFont.systemFont(ofSize: 11), .foregroundColor: NSColor.black, .backgroundColor: NSColor.white])
        NSGraphicsContext.restoreGraphicsState()
        guard let data = bitmap.representation(using: .png, properties: [:]) else { throw SourceFailure.unavailable }
        snapshots = snapshots.filter { $0.value.0 > Date() }
        guard snapshots.count < 16 else { throw SourceFailure.unavailable }
        let digest = SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
        let handle = "maps-snapshot:" + UUID().uuidString
        snapshots[handle] = (Date().addingTimeInterval(120), data, digest)
        let blob = QueryBlob(digest: digest, size: UInt64(data.count), handle: handle)
        return QueryOutput(result: .object(["image": blob.reference, "attribution": .string("© Apple Maps")]), blobs: [blob])
    }

    @MainActor
    public static func run(_ operation: String, args: JSON) async throws -> JSON {
        try Task.checkCancellation()
        guard active < 8 else { throw SourceFailure.unavailable }
        active += 1
        defer { active -= 1 }
        if operation == "location.current" {
            try fields(args, [])
            let request = CurrentFix()
            let location = try await request.sample()
            var value = coordinate(location.coordinate).object
            value["accuracy"] = .number(location.horizontalAccuracy)
            value["timestamp"] = .string(timestamp(location.timestamp))
            value["source"] = .string("mac")
            return .object(value)
        }
        if operation == "location.geocode" || operation == "location.reverse_geocode" {
            let marks: [MKMapItem]
            if operation == "location.geocode" {
                try fields(args, ["address"])
                guard !args["address"].string.isEmpty, args["address"].string.utf8.count <= 1024,
                      let request = MKGeocodingRequest(addressString: args["address"].string) else { throw SourceFailure.invalid }
                let cancellation = NativeCancellation { request.cancel() }
                marks = try await withTaskCancellationHandler { try await request.mapItems } onCancel: { cancellation.cancel() }
            } else {
                try fields(args, ["lat", "lon"])
                let point = try point(args)
                guard let request = MKReverseGeocodingRequest(location: CLLocation(latitude: point.latitude, longitude: point.longitude)) else { throw SourceFailure.invalid }
                let cancellation = NativeCancellation { request.cancel() }
                marks = try await withTaskCancellationHandler { try await request.mapItems } onCancel: { cancellation.cancel() }
            }
            try Task.checkCancellation()
            return .object(["places": .array(marks.map { mark in
                var value = coordinate(mark.location.coordinate).object
                value["address"] = .string(mark.address?.fullAddress ?? "")
                return .object(value)
            })])
        }
        if operation == "maps.search" || operation == "maps.nearby" {
            let textKey = operation == "maps.search" ? "query" : "category"
            try fields(args, [textKey, "near", "radius"])
            let center = try point(args["near"])
            let radius = args["radius"] == .null ? 5000 : args["radius"].number
            let text = args[textKey].string
            guard !text.isEmpty, text.utf8.count <= 512, radius > 0, radius <= 100000 else { throw SourceFailure.invalid }
            let request = MKLocalSearch.Request()
            request.naturalLanguageQuery = text
            request.region = MKCoordinateRegion(center: center, latitudinalMeters: radius * 2, longitudinalMeters: radius * 2)
            let search = MKLocalSearch(request: request)
            let cancellation = NativeCancellation { search.cancel() }
            let response = try await withTaskCancellationHandler { try await search.start() } onCancel: { cancellation.cancel() }
            try Task.checkCancellation()
            let origin = CLLocation(latitude: center.latitude, longitude: center.longitude)
            let places = response.mapItems.filter { origin.distance(from: CLLocation(latitude: $0.location.coordinate.latitude, longitude: $0.location.coordinate.longitude)) <= radius }.prefix(50).map { item -> JSON in
                var value = coordinate(item.location.coordinate).object
                value["name"] = .string(item.name ?? "")
                value["address"] = .string(item.address?.fullAddress ?? "")
                value["category"] = .string(item.pointOfInterestCategory?.rawValue ?? "")
                value["phone"] = .string(item.phoneNumber ?? "")
                value["url"] = .string(item.url?.absoluteString ?? "")
                return .object(value)
            }
            return .object(["places": .array(Array(places))])
        }
        guard operation == "maps.directions" || operation == "maps.eta" else { throw SourceFailure.unsupported }
        try fields(args, ["from", "to", "mode"])
        let from = try point(args["from"])
        let to = try point(args["to"])
        let mode = args["mode"].string
        let transport: MKDirectionsTransportType
        switch mode {
        case "driving": transport = .automobile
        case "walking": transport = .walking
        case "transit":
            guard operation == "maps.eta" else { throw SourceFailure.unsupported }
            transport = .transit
        case "cycling": transport = .cycling
        default: throw SourceFailure.invalid
        }
        let request = MKDirections.Request()
        request.source = MKMapItem(location: CLLocation(latitude: from.latitude, longitude: from.longitude), address: nil)
        request.destination = MKMapItem(location: CLLocation(latitude: to.latitude, longitude: to.longitude), address: nil)
        request.transportType = transport
        let directions = MKDirections(request: request)
        let cancellation = NativeCancellation { directions.cancel() }
        if operation == "maps.eta" {
            let estimate = try await withTaskCancellationHandler { try await directions.calculateETA() } onCancel: { cancellation.cancel() }
            try Task.checkCancellation()
            return .object(["distance": .number(estimate.distance), "duration": .number(estimate.expectedTravelTime * 1000)])
        }
        let response = try await withTaskCancellationHandler { try await directions.calculate() } onCancel: { cancellation.cancel() }
        try Task.checkCancellation()
        guard response.routes.prefix(3).allSatisfy({ $0.polyline.pointCount <= 10000 }) else { throw SourceFailure.invalid }
        return .object(["routes": .array(response.routes.prefix(3).map { route in
            var value: [String: JSON] = ["distance": .number(route.distance), "duration": .number(route.expectedTravelTime * 1000)]
            if operation == "maps.directions" {
                value["steps"] = .array(route.steps.map { .object(["instructions": .string($0.instructions), "distance": .number($0.distance)]) })
                let line = route.polyline
                value["polyline"] = .array((0..<line.pointCount).map { coordinate(line.points()[$0].coordinate) })
            }
            return .object(value)
        })])
    }

    private static func fields(_ value: JSON, _ allowed: Set<String>) throws {
        guard case .object = value, Set(value.object.keys).isSubset(of: allowed) else { throw SourceFailure.invalid }
    }

    private static func point(_ value: JSON) throws -> CLLocationCoordinate2D {
        try fields(value, ["lat", "lon"])
        guard value["lat"] != .null, value["lon"] != .null else { throw SourceFailure.invalid }
        let lat = value["lat"].number
        let lon = value["lon"].number
        guard lat.isFinite, lon.isFinite, abs(lat) <= 90, abs(lon) <= 180,
              value["lat"] == .number(lat), value["lon"] == .number(lon) else { throw SourceFailure.invalid }
        return CLLocationCoordinate2D(latitude: lat, longitude: lon)
    }

    private static func coordinate(_ value: CLLocationCoordinate2D) -> JSON {
        .object(["lat": .number(value.latitude), "lon": .number(value.longitude)])
    }
}

@MainActor
private final class CurrentFix: NSObject, @preconcurrency CLLocationManagerDelegate {
    private let manager = CLLocationManager()
    private var pending: CheckedContinuation<CLLocation, any Error>?

    func sample() async throws -> CLLocation {
        guard manager.authorizationStatus == .authorizedAlways else { throw SourceFailure.permissionMissing }
        return try await withTaskCancellationHandler {
            try Task.checkCancellation()
            return try await withCheckedThrowingContinuation { continuation in
                pending = continuation
                manager.delegate = self
                manager.requestLocation()
            }
        } onCancel: {
            Task { @MainActor in self.finish(.failure(CancellationError())) }
        }
    }

    func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        guard let location = locations.last, location.horizontalAccuracy >= 0, abs(location.timestamp.timeIntervalSinceNow) < 120 else {
            finish(.failure(SourceFailure.unavailable))
            return
        }
        finish(.success(location))
    }

    func locationManager(_ manager: CLLocationManager, didFailWithError error: any Error) {
        finish(.failure(SourceFailure.unavailable))
    }

    private func finish(_ result: Result<CLLocation, any Error>) {
        manager.stopUpdatingLocation()
        manager.delegate = nil
        let continuation = pending
        pending = nil
        continuation?.resume(with: result)
    }
}

@MainActor
private final class NativeCancellation {
    private let action: @MainActor () -> Void
    init(_ action: @escaping @MainActor () -> Void) { self.action = action }
    nonisolated func cancel() { Task { @MainActor in self.action() } }
}
