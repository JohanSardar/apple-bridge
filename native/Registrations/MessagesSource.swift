@MainActor
extension SessionSources {
    static func registerMessagesSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        let messages = Messages(diagnostics: labEnabled ? home.appendingPathComponent("messages-watch.json") : nil, labCommands: labEnabled)
        try? await peer.register(messages, readBlob: { handle, offset, length in
            try await messages.readBlob(handle: handle, offset: offset, length: length)
        })
    }
}
