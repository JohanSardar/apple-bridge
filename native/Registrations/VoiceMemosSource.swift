@MainActor
extension SessionSources {
    static func registerVoiceMemosSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        let source = VoiceMemosSource()
        try? await peer.register(source, blobPrefix: "voice-memo-blob:", readBlob: { handle, offset, length in
            try await source.readBlob(handle, offset: offset, length: length)
        })
    }
}
