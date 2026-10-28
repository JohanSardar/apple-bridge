@MainActor
extension SessionSources {
    static func registerMusicSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        let source = MusicPlayer()
        try? await peer.register(source)
        try? await peer.registerQuery("music.now_playing") { args in
            guard args.object.isEmpty, args == .object([:]) else { throw SourceFailure.invalid }
            let player = try await source.nowPlaying()
            return .object(player.object.filter { $0.key != "id" })
        }
        try? await peer.registerQuery("music.search_catalog") { args in try await MusicCatalog.search(args) }
    }
}
