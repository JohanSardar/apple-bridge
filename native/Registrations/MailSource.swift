@MainActor
extension SessionSources {
    static func registerMailSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        try? await peer.register(MailSource())
    }
}
