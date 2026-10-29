@MainActor
extension SessionSources {
    static func registerWeatherSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        for query in ["weather.current", "weather.hourly", "weather.daily"] {
            try? await peer.registerQuery(query) { args in try await WeatherQueries.run(query, args: args) }
        }
    }
}
