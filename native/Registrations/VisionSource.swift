@MainActor
extension SessionSources {
    static func registerVisionSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        try? await peer.registerQuery("vision.ocr") { args in try await VisionQueries.run("vision.ocr", args: args) }
    }
}
