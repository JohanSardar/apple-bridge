@MainActor
extension SessionSources {
    static func registerSafariSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        try? await peer.register(SafariSource(cacheURL: home.appendingPathComponent("safari-capture.json")))
    }
}
