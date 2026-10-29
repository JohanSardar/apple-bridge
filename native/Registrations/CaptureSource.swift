@MainActor
extension SessionSources {
    static func registerCaptureSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        try? await peer.register(capturePlayback())
    }
}
