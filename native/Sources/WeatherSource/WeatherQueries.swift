import Foundation
import CoreLocation
import WeatherKit
import SourceSupport

public enum WeatherQueries {
    public static func run(_ query: String, args: JSON) async throws -> JSON {
        guard ["weather.current", "weather.hourly", "weather.daily"].contains(query),
              Set(args.object.keys) == Set(["latitude", "longitude"]),
              case .number(let latitude) = args["latitude"],
              case .number(let longitude) = args["longitude"],
              latitude.isFinite, longitude.isFinite, abs(latitude) <= 90, abs(longitude) <= 180 else { throw SourceFailure.invalid }
        let location = CLLocation(latitude: latitude, longitude: longitude)
        let service = WeatherService.shared
        let attribution = try await service.attribution
        try Task.checkCancellation()
        let samples: [JSON]
        if query == "weather.current" {
            let weather = try await service.weather(for: location, including: .current)
            samples = [sample(weather.date, weather.temperature, weather.condition)]
        } else if query == "weather.hourly" {
            let weather = try await service.weather(for: location, including: .hourly)
            samples = weather.prefix(48).map { sample($0.date, $0.temperature, $0.condition) }
        } else {
            let weather = try await service.weather(for: location, including: .daily)
            samples = weather.prefix(10).map {
                var value = sample($0.date, $0.highTemperature, $0.condition).object
                value["low_temperature_celsius"] = .number($0.lowTemperature.converted(to: .celsius).value)
                return .object(value)
            }
        }
        try Task.checkCancellation()
        return .object([
            "query": .string(query),
            "temperature_unit": .string("celsius"),
            "attribution": .object([
                "provider": .string("Apple Weather"),
                "legal_url": .string(attribution.legalPageURL.absoluteString),
                "combined_mark_light_url": .string(attribution.combinedMarkLightURL.absoluteString),
                "combined_mark_dark_url": .string(attribution.combinedMarkDarkURL.absoluteString)
            ]),
            "samples": .array(samples)
        ])
    }

    private static func sample(_ date: Date, _ temperature: Measurement<UnitTemperature>, _ condition: WeatherCondition) -> JSON {
        .object([
            "time": .string(ISO8601DateFormatter().string(from: date)),
            "temperature_celsius": .number(temperature.converted(to: .celsius).value),
            "condition": .string(condition.rawValue)
        ])
    }
}
