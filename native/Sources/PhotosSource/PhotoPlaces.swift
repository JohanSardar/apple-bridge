import Foundation
import CoreLocation
import MapKit
import SourceSupport

struct PhotoPlace: Sendable, Equatable {
    let name: String
    let city: String?
    let state: String?
    let country: String?

    var json: JSON {
        .object(["place": .string(name), "city": city.map(JSON.string) ?? .null, "state": state.map(JSON.string) ?? .null, "country": country.map(JSON.string) ?? .null])
    }
}

struct PlaceCell: Hashable, Sendable {
    let lat: Int
    let lon: Int

    init?(_ location: CLLocation?) {
        guard let coordinate = location?.coordinate, CLLocationCoordinate2DIsValid(coordinate), coordinate.latitude != 0 || coordinate.longitude != 0 else { return nil }
        lat = Int((coordinate.latitude * 20).rounded(.down))
        lon = Int((coordinate.longitude * 20).rounded(.down))
    }
}

actor PhotoPlaces {
    private var named: [PlaceCell: PhotoPlace] = [:]
    private var unnamed: Set<PlaceCell> = []
    private var pausedUntil: ContinuousClock.Instant?

    func cached(_ cell: PlaceCell) -> PhotoPlace? { named[cell] }

    func known(_ cell: PlaceCell) -> Bool { named[cell] != nil || unnamed.contains(cell) }

    func resolve(_ spots: [(PlaceCell, CLLocation)], budget: Duration) async -> Bool {
        let clock = ContinuousClock()
        let stop = clock.now + budget
        for (cell, location) in spots where !known(cell) {
            if Task.isCancelled || clock.now >= stop { return false }
            if let pausedUntil, clock.now < pausedUntil { return false }
            guard let request = MKReverseGeocodingRequest(location: location) else { unnamed.insert(cell); continue }
            do {
                let items = try await request.mapItems
                if named.count + unnamed.count >= 50_000 { named.removeAll(); unnamed.removeAll() }
                if let place = items.lazy.compactMap(Self.place).first { named[cell] = place } else { unnamed.insert(cell) }
            } catch let error as MKError where error.code == .placemarkNotFound {
                unnamed.insert(cell)
            } catch {
                pausedUntil = clock.now + .seconds(60)
                return false
            }
        }
        return true
    }

    private static func place(_ item: MKMapItem) -> PhotoPlace? {
        guard let names = item.addressRepresentations else { return nil }
        let country = names.regionName.flatMap { $0.isEmpty ? nil : $0 }
        let city = names.cityName.flatMap { $0.isEmpty ? nil : $0 }
        var state: String?
        if country == "United States" || country == nil, let context = names.cityWithContext(.short), let code = context.split(separator: ",").last?.trimmingCharacters(in: .whitespaces) {
            state = usStates[code]
        }
        if state == nil, country == "United States" || country == nil, let address = names.fullAddress(includingRegion: true, singleLine: true) {
            state = address.components(separatedBy: ", ").reversed().lazy.compactMap { part -> String? in
                let words = part.split(separator: " ")
                guard words.count == 2, words[1].allSatisfy(\.isNumber) else { return nil }
                return usStates[String(words[0])]
            }.first
        }
        let full = names.cityWithContext(.full)
        let name: String
        if let state {
            name = [city, state, country ?? "United States"].compactMap { $0 }.joined(separator: ", ")
        } else if let full, full.contains(", "), let country, full.hasSuffix(country) {
            name = full
        } else {
            name = [city, country].compactMap { $0 }.joined(separator: ", ")
        }
        guard !name.isEmpty else { return nil }
        return PhotoPlace(name: name, city: city, state: state, country: state == nil ? country : country ?? "United States")
    }

    private static let usStates: [String: String] = [
        "AL": "Alabama", "AK": "Alaska", "AZ": "Arizona", "AR": "Arkansas", "CA": "California", "CO": "Colorado", "CT": "Connecticut", "DE": "Delaware", "DC": "District of Columbia", "FL": "Florida", "GA": "Georgia", "HI": "Hawaii", "ID": "Idaho", "IL": "Illinois", "IN": "Indiana", "IA": "Iowa", "KS": "Kansas", "KY": "Kentucky", "LA": "Louisiana", "ME": "Maine", "MD": "Maryland", "MA": "Massachusetts", "MI": "Michigan", "MN": "Minnesota", "MS": "Mississippi", "MO": "Missouri", "MT": "Montana", "NE": "Nebraska", "NV": "Nevada", "NH": "New Hampshire", "NJ": "New Jersey", "NM": "New Mexico", "NY": "New York", "NC": "North Carolina", "ND": "North Dakota", "OH": "Ohio", "OK": "Oklahoma", "OR": "Oregon", "PA": "Pennsylvania", "RI": "Rhode Island", "SC": "South Carolina", "SD": "South Dakota", "TN": "Tennessee", "TX": "Texas", "UT": "Utah", "VT": "Vermont", "VA": "Virginia", "WA": "Washington", "WV": "West Virginia", "WI": "Wisconsin", "WY": "Wyoming", "PR": "Puerto Rico",
    ]
}
