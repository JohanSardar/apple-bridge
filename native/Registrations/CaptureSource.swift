@MainActor
extension SessionSources {
    static func registerCaptureSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        let capture = CaptureSource(directory: home.appendingPathComponent("capture-artifacts"))
        try? await peer.register(capture)
        try? await peer.registerBlobReader("capture-artifact:", source: "capture") { handle, offset, length in
            try await capture.readBlob(handle, offset: offset, length: length)
        }
    }
}
