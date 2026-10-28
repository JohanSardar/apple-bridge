@MainActor
extension SessionSources {
    static func registerAudioSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        try? await peer.registerQuery("audio.transcribe") { args in try await AudioQueries.run("audio.transcribe", args: args) }
    }
}
