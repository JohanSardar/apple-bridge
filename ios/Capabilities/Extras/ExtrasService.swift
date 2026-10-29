import Foundation

struct ExtrasRequest: Codable {
    var device_id: String
    var latitude: Double?
    var longitude: Double?
    var venue_id: UUID?
}

enum ExtrasError: Error {
    case disabled
    case unavailable
    case invalidArguments
    case needsSelection
}

@MainActor
protocol ExtrasAdapter {
    func weather(latitude: Double, longitude: Double) async throws -> Data
    func journal() async throws -> Data
    func energy(venue: UUID) async throws -> Data
}

@MainActor
enum ExtrasService {
    static func execute(_ name: String, raw: Data, enabled: Bool, granted: Bool, adapter: any ExtrasAdapter) async throws -> Data {
        guard enabled else { throw ExtrasError.disabled }
        guard granted else { throw ExtrasError.unavailable }
        let args = try JSONDecoder().decode(ExtrasRequest.self, from: raw)
        switch name {
        case "iphone.weather":
            guard let lat = args.latitude, let lon = args.longitude, lat.isFinite, lon.isFinite,
                  (-90...90).contains(lat), (-180...180).contains(lon) else { throw ExtrasError.invalidArguments }
            return try await adapter.weather(latitude: lat, longitude: lon)
        case "iphone.journal.suggestions":
            return try await adapter.journal()
        case "iphone.energy.hints":
            guard let venue = args.venue_id else { throw ExtrasError.invalidArguments }
            return try await adapter.energy(venue: venue)
        default:
            throw ExtrasError.invalidArguments
        }
    }
}
