@MainActor
extension SessionSources {
    static func registerFocusSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        try? await peer.register(FocusSource())
    }
}
