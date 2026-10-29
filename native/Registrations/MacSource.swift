@MainActor
extension SessionSources {
    static func registerMacSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        try? await peer.register(MacSource())
        for op in ["mac.bluetooth_devices", "mac.airplay_routes", "mac.time_machine_status", "mac.recognize_song"] {
            try? await peer.registerQuery(op) { args in try await MacExtrasQueries.run(op, args: args) }
        }
        try? await peer.registerQuery("mac.network_check") { args in try await MacNetworkQueries.check(args) }
        try? await peer.registerQuery("mac.clipboard_read") { args in try await MacClipboardQueries.read(args) }
    }
}
