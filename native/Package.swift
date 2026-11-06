// swift-tools-version:6.3
import PackageDescription

let sourceTargets: [Target] = [
    .target(name: "FocusSource", dependencies: ["SourceSupport"]),
    .target(name: "CaptureSource", dependencies: ["SourceSupport", "SpeechSource"]),
    .target(name: "SpeechSource", dependencies: ["SourceSupport"]),
    .target(name: "AudioSource", dependencies: ["SourceSupport"]),
    .target(name: "MapsSource", dependencies: ["SourceSupport"]),
    .target(name: "MacSource", dependencies: ["SourceSupport"]),
    .target(name: "MailSource", dependencies: ["SourceSupport"]),
    .target(name: "FilesSource", dependencies: ["SourceSupport"]),
    .target(name: "VisionSource", dependencies: ["SourceSupport"]),
    .target(name: "PDFSource", dependencies: ["SourceSupport"]),
    .target(name: "ContactsSource", dependencies: ["SourceSupport", "ContactsHistory", "MediaDerivatives"]),
    .target(name: "CalendarSource", dependencies: ["AgendaGate", "SourceSupport"]),
    .target(name: "RemindersSource", dependencies: ["AgendaGate", "SourceSupport"]),
    .target(name: "NotesSource", dependencies: ["SourceSupport"]),
    .target(name: "MessagesSource", dependencies: ["SourceSupport"]),
    .target(name: "SafariSource", dependencies: ["SourceSupport"]),
]

let package = Package(
    name: "AppleBridgeHelper",
    platforms: [.macOS("26.0")],
    products: [.executable(name: "AppleBridgeHelper", targets: ["AppleBridgeHelper"])],
    targets: sourceTargets + [
        .target(name: "SourceSupport"),
        .target(name: "NativeProtocol", dependencies: ["SourceSupport"]),
        .executableTarget(name: "MessagesSafetyChecks", dependencies: ["MessagesSource", "SourceSupport"]),
        .executableTarget(name: "CaptureChecks", dependencies: ["CaptureSource", "SourceSupport"]),
        .executableTarget(name: "FilesChecks", dependencies: ["FilesSource", "SourceSupport"]),
        .executableTarget(name: "MacChecks", dependencies: ["MacSource", "SourceSupport"]),
        .executableTarget(name: "SourceSupportChecks", dependencies: ["SourceSupport"]),
        .target(name: "ContactsHistory", publicHeadersPath: "include"),
        .target(name: "MediaExportGuard", publicHeadersPath: "include"),
        .target(name: "MediaDerivatives", dependencies: ["MediaExportGuard"]),
        .executableTarget(name: "MediaChecks", dependencies: ["MediaDerivatives", "MediaExportGuard"]),
        .target(name: "AgendaGate", dependencies: ["SourceSupport"]),
        .target(name: "AppleSession", dependencies: sourceTargets.map { .target(name: $0.name) } + ["AgendaGate", "SourceSupport", "MediaDerivatives", "NativeProtocol"]),
        .executableTarget(name: "NativeChecks", dependencies: ["NativeProtocol", "SourceSupport"]),
        .executableTarget(name: "HelperIdentityChecks", dependencies: ["AppleSession", "SourceSupport"]),
        .executableTarget(name: "HelperIdentityFixture"),
        .executableTarget(name: "SessionChecks", dependencies: ["AppleSession", "NativeProtocol", "RemindersSource", "SourceSupport"]),
        .executableTarget(name: "CalendarChecks", dependencies: ["CalendarSource"], path: "Tests/CalendarChecks"),
        .executableTarget(name: "AppleBridgeHelper", dependencies: ["AppleSession", "MediaDerivatives", "SourceSupport"])
    ]
)
