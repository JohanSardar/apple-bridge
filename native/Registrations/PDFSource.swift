@MainActor
extension SessionSources {
    static func registerPDFSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        if let source = try? PDFSource(home: home) {
            try? await peer.register(source)
        }
        try? await peer.registerQuery("pdf.text") { args in try await PDFQueries.run("pdf.text", args: args) }
        try? await peer.registerQuery("pdf.search") { args in try await PDFQueries.run("pdf.search", args: args) }
    }
}
