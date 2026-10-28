@MainActor
extension SessionSources {
    static func registerMusicSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        try? await peer.registerQuery("music.search_catalog") { args in try await MusicCatalog.search(args) }
    }
}
