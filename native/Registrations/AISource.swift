@MainActor
extension SessionSources {
    static func registerAISource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        try? await peer.registerQuery("ai.local") { args in try await AIQueries.run("ai.local", args: args) }
        try? await peer.registerQuery("ai.rewrite") { args in try await AIQueries.run("ai.rewrite", args: args) }
    }
}
