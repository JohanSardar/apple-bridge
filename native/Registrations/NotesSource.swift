@MainActor
extension SessionSources {
    static func registerNotesSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        try? await peer.register(NotesSource())
    }
}
