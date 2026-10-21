// swift-tools-version:6.3
import PackageDescription

let package = Package(
    name: "AppleBridgeHelper",
    platforms: [.macOS("26.0")],
    products: [.executable(name: "AppleBridgeHelper", targets: ["AppleBridgeHelper"])],
    targets: [
        .target(name: "SourceSupport"),
        .target(name: "ContactsHistory", publicHeadersPath: "include"),
        .target(name: "ContactsSource", dependencies: ["SourceSupport", "ContactsHistory"]),
        .target(name: "Decoders"),
        .target(name: "MediaDerivatives"),
        .executableTarget(name: "MediaChecks", dependencies: ["MediaDerivatives"]),
        .target(name: "UsageSource", dependencies: ["SourceSupport", "Decoders"]),
        .target(name: "AgendaGate", dependencies: ["SourceSupport"]),
        .target(name: "CalendarSource", dependencies: ["AgendaGate", "SourceSupport"]),
        .target(name: "RemindersSource", dependencies: ["AgendaGate", "SourceSupport"]),
        .target(name: "NotesSource", dependencies: ["SourceSupport", "Decoders"]),
        .target(name: "MessagesSource", dependencies: ["SourceSupport", "Decoders", "MediaDerivatives"]),
        .executableTarget(name: "UsageChecks", dependencies: ["Decoders", "UsageSource", "SourceSupport"]),
        .executableTarget(name: "NotesDecoderChecks", dependencies: ["Decoders"]),
        .executableTarget(name: "DecoderChecks", dependencies: ["Decoders"]),
        .executableTarget(name: "MessagesCaptureChecks", dependencies: ["MessagesSource", "SourceSupport"]),
        .executableTarget(name: "MessagesBench", dependencies: ["MessagesSource", "SourceSupport"]),
        .target(name: "AppleSession", dependencies: ["UsageSource", "NotesSource", "CalendarSource", "ContactsSource", "MessagesSource", "RemindersSource", "AgendaGate", "SourceSupport"]),
        .executableTarget(name: "CalendarChecks", dependencies: ["CalendarSource"], path: "Tests/CalendarChecks"),
        .executableTarget(name: "CommandOriginChecks", dependencies: ["SourceSupport"], path: "Tests/CommandOriginChecks"),
        .executableTarget(name: "AppleBridgeHelper", dependencies: ["AppleSession"])
    ]
)
