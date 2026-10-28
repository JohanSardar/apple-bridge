@MainActor
extension SessionSources {
    static func registerFilesSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        guard let source = try? FilesSource(home: home) else { return }
        try? await peer.register(source)
        try? await peer.registerQuery("files.search") { args in try await source.search(args) }
        try? await peer.registerQuery("files.read") { args in try await source.read(args) }
    }
}
