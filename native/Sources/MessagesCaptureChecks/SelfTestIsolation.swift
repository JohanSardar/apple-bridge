import MessagesSource

struct SelfTestIsolationChecks {
    static func run() throws {
        let cases: [(String, [String], [String], [String])] = [
            ("national phone already participates", ["2025550123"], ["+1 (202) 555-0123"], []),
            ("country phone already participates", ["+12025550123"], ["202-555-0123"], []),
            ("international prefix", ["00442079460000"], ["+44 20 7946 0000"], []),
            ("national trunk", ["02079460000"], ["+442079460000"], []),
            ("all service prefixes", ["+12025550123"], ["SMS:2025550123", "RCS;-;2025550123"], []),
            ("email prefix and case", ["Self@Example.com"], ["iMessage:E:mailto:self%40example.com"], []),
            ("distinct email", ["unused@example.com"], ["self@example.com"], ["unused@example.com"]),
            ("distinct phone", ["+12025550123"], ["+442079460000"], ["+12025550123"]),
            ("empty chat inventory", ["self@example.com"], [], ["self@example.com"]),
            ("invalid alias refused", ["", "+12", "unknown", "a@@b"], [], [])
        ]
        for (name, own, participants, expected) in cases {
            try MessagesCaptureChecks.require(MessagesSelfTestIsolation.unused(own, participants: participants) == expected, name)
        }
        print("self-test isolation: \(cases.count) checks passed; zero native writes")
    }
}
