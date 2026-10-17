// swift-tools-version:6.3
import PackageDescription

let package = Package(
    name: "AppleBridgeHelper",
    platforms: [.macOS("26.0")],
    products: [.executable(name: "AppleBridgeHelper", targets: ["AppleBridgeHelper"])],
    targets: [
        .target(name: "SourceSupport"),
        .target(name: "Decoders"),
        .target(name: "AgendaGate", dependencies: ["SourceSupport"]),
        .target(name: "RemindersSource", dependencies: ["AgendaGate", "SourceSupport"]),
        .target(name: "MessagesSource", dependencies: ["SourceSupport", "Decoders"]),
        .executableTarget(name: "DecoderChecks", dependencies: ["Decoders"]),
        .executableTarget(name: "MessagesCaptureChecks", dependencies: ["MessagesSource", "SourceSupport"]),
        .executableTarget(name: "MessagesBench", dependencies: ["MessagesSource", "SourceSupport"]),
        .target(name: "AppleSession", dependencies: ["MessagesSource", "RemindersSource", "AgendaGate", "SourceSupport"]),
        .executableTarget(name: "AppleBridgeHelper", dependencies: ["AppleSession"])
    ]
)
