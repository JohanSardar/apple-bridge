import Foundation
import WeatherKit
import CoreLocation
import EnergyKit

@MainActor
enum ExtrasCapability {
    static let descriptors = [
        descriptor("weather", title: "WeatherKit", command: "iphone.weather", granted: true, permission: "Available; coordinates supplied by gateway"),
        descriptor("journal", title: "Journal suggestions", command: "iphone.journal.suggestions", granted: journalGranted, permission: journalGranted ? "Choose a suggestion on this phone" : "limited: needs Journaling Suggestions capability"),
        descriptor("energy", title: "EnergyKit hints", command: "iphone.energy.hints", granted: energyGranted, permission: energyGranted ? "Needs an authorized energy venue" : "limited: needs EnergyKit capability"),
    ]
    static var journalGranted: Bool { Bundle.main.object(forInfoDictionaryKey: "ABJournalGranted") as? Bool == true }
    static var energyGranted: Bool { Bundle.main.object(forInfoDictionaryKey: "ABEnergyGranted") as? Bool == true }

    static func descriptor(_ id: String, title: String, command: String, granted: Bool, permission: String) -> Capability {
        Capability(id: id, title: title, permission: permission, available: granted, handlers: [command: { raw in
            try await ExtrasService.execute(command, raw: raw, enabled: UserDefaults.standard.bool(forKey: "capability.\(id).enabled"), granted: granted, adapter: PhoneExtrasAdapter())
        }])
    }
    static var selectedSuggestion: String?
}

@MainActor
struct PhoneExtrasAdapter: ExtrasAdapter {
    func weather(latitude: Double, longitude: Double) async throws -> Data {
        let weather = try await WeatherService.shared.weather(for: CLLocation(latitude: latitude, longitude: longitude))
        let attribution = try await WeatherService.shared.attribution
        return try JSONSerialization.data(withJSONObject: [
            "temperature_celsius": weather.currentWeather.temperature.converted(to: .celsius).value,
            "condition": weather.currentWeather.condition.description,
            "attribution_url": attribution.legalPageURL.absoluteString,
            "attribution_mark_light_url": attribution.combinedMarkLightURL.absoluteString,
            "attribution_mark_dark_url": attribution.combinedMarkDarkURL.absoluteString,
        ])
    }
    func journal() async throws -> Data {
        guard let title = ExtrasCapability.selectedSuggestion else { throw ExtrasError.needsSelection }
        return try JSONSerialization.data(withJSONObject: ["title": title])
    }
    func energy(venue: UUID) async throws -> Data {
        for try await guidance in ElectricityGuidance.sharedService.guidance(using: .init(suggestedAction: .shift), at: venue) {
            return try JSONEncoder().encode(guidance)
        }
        throw ExtrasError.unavailable
    }
}
