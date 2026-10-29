@MainActor
extension SessionSources {
    static func registerSpeechSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        try? await peer.register(SpeechPlayback())
        try? await peer.registerQuery("speech.synthesize") { args in try await SpeechQueries.run("speech.synthesize", args: args) }
    }
}
