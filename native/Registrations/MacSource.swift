@MainActor
extension SessionSources {
    static func registerMacSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        try? await peer.register(MacSource())
    }
}
