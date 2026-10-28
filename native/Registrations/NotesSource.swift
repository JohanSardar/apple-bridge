@MainActor
extension SessionSources {
    static func registerNotesSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        try? await peer.register(try NotesSource(home: home, trace: CaptureTrace(url: labEnabled ? home.appendingPathComponent("capture-trace.jsonl") : nil)))
    }
}
