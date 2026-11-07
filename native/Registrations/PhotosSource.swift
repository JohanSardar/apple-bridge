@MainActor
extension SessionSources {
    static func registerPhotosSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        let photos = PhotosSource(home: home)
        try? await peer.register(photos)
        for operation in PhotosSource.operations {
            try? await peer.registerQuery(operation) { args in try await photos.query(operation, args: args) }
        }
    }
}
