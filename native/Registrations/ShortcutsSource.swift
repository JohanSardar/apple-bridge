@MainActor
extension SessionSources {
    static func registerShortcutsSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        try? await peer.register(ShortcutsSource())
    }
}
