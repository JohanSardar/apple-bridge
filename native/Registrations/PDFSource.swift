@MainActor
extension SessionSources {
    static func registerPDFSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        try? await peer.registerQuery("pdf.text") { args in try await PDFQueries.run("pdf.text", args: args) }
        try? await peer.registerQuery("pdf.search") { args in try await PDFQueries.run("pdf.search", args: args) }
    }
}
