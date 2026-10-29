@MainActor
extension SessionSources {
    static func registerShortcutsSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        try? await peer.register(ShortcutsSource())
        try? await peer.registerQuery("shortcuts.actions") { args in
            guard case .object(let fields) = args, fields.isEmpty else { throw SourceFailure.invalid }
            return try ActionCatalog.discover()
        }
    }
}
