@MainActor
extension SessionSources {
    static func registerCallsSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        try? await peer.register(CallsSource())
    }
}
