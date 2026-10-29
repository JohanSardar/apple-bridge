@MainActor
extension SessionSources {
 static func registerVoiceMemosSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
 try? await peer.register(VoiceMemosSource())
 }
}
