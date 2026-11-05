@MainActor
extension SessionSources {
    static func registerFilesSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        try? await peer.register(FilesSource())
    }
}
