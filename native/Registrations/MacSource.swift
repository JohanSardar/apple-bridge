@MainActor
extension SessionSources {
    static func registerMacSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        try? await peer.register(MacSource())
        try? await peer.registerQuery("mac.network_check") { args in try await MacNetworkQueries.check(args) }
        try? await peer.registerQuery("mac.clipboard_read") { args in try await MacClipboardQueries.read(args) }
    }
}
