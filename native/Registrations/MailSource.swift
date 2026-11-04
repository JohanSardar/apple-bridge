@MainActor
extension SessionSources {
    static func registerMailSource(into peer: AppleSession, home: URL, labEnabled: Bool) async {
        try? await peer.register(MailSource())
        if CaptureOwnership(home: home).swiftCaptures("mail") {
            try? await peer.registerQuery("mail.body") { args in try MailBody.read(args) }
        }
    }
}
