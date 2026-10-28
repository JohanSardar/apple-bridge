@MainActor
extension SessionSources {
    static func registerPhotosSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        let source = PhotosSource()
        try? await peer.register(source)
        try? await peer.registerQuery("photos.search") { args in try await source.search(args) }
        try? await peer.registerQuery("photos.original") { args in try await source.blob("photos.original", args) }
        try? await peer.registerQuery("photos.thumbnail") { args in try await source.blob("photos.thumbnail", args) }
    }
}
