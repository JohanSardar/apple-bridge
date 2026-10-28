// swift-tools-version:6.3
import PackageDescription

let sourceTargets: [Target] = [
    .target(name: "MacSource", dependencies: ["SourceSupport"]),
    .target(name: "MailSource", dependencies: ["SourceSupport"]),
    .target(name: "FilesSource", dependencies: ["SourceSupport"]),
    .target(name: "VisionSource", dependencies: ["SourceSupport"]),
    .target(name: "PDFSource", dependencies: ["SourceSupport"]),
    .target(name: "ShortcutsSource", dependencies: ["SourceSupport"]),
    .target(name: "ContactsSource", dependencies: ["SourceSupport", "ContactsHistory", "MediaDerivatives"]),
    .target(name: "UsageSource", dependencies: ["SourceSupport", "Decoders"]),
    .target(name: "CalendarSource", dependencies: ["AgendaGate", "SourceSupport"]),
    .target(name: "RemindersSource", dependencies: ["AgendaGate", "SourceSupport", "RemindersStore"]),
    .target(name: "NotesSource", dependencies: ["SourceSupport", "Decoders", "MediaDerivatives"]),
    .target(name: "MessagesSource", dependencies: ["SourceSupport", "Decoders", "MediaDerivatives"]),
]

let package = Package(
    name: "AppleBridgeHelper",
    platforms: [.macOS("26.0")],
    products: [.executable(name: "AppleBridgeHelper", targets: ["AppleBridgeHelper"])],
    targets: sourceTargets + [
        .target(name: "SourceSupport"),
        .executableTarget(name: "MailChecks", dependencies: ["MailSource", "SourceSupport"]),
        .executableTarget(name: "FilesChecks", dependencies: ["FilesSource", "SourceSupport"]),
        .executableTarget(name: "SourceSupportChecks", dependencies: ["SourceSupport"]),
        .executableTarget(name: "ShortcutsChecks", dependencies: ["ShortcutsSource", "SourceSupport"]),
        .target(name: "ContactsHistory", publicHeadersPath: "include"),
        .target(name: "Decoders"),
        .target(name: "MediaExportGuard", publicHeadersPath: "include"),
        .target(name: "MediaDerivatives", dependencies: ["MediaExportGuard"]),
        .executableTarget(name: "MediaChecks", dependencies: ["MediaDerivatives", "MediaExportGuard"]),
        .target(name: "AgendaGate", dependencies: ["SourceSupport"]),
        .target(name: "RemindersStore", dependencies: ["SourceSupport"]),
        .executableTarget(name: "RemindersStoreChecks", dependencies: ["RemindersStore", "SourceSupport"], path: "Tests/RemindersStoreChecks", exclude: ["Fixtures"]),
        .executableTarget(name: "UsageChecks", dependencies: ["Decoders", "UsageSource", "SourceSupport"]),
        .executableTarget(name: "NotesDecoderChecks", dependencies: ["Decoders", "NotesSource", "SourceSupport"]),
        .executableTarget(name: "DecoderChecks", dependencies: ["Decoders"]),
        .executableTarget(name: "MessagesCaptureChecks", dependencies: ["MessagesSource", "SourceSupport", "MediaDerivatives"]),
        .executableTarget(name: "MessagesBench", dependencies: ["MessagesSource", "SourceSupport"]),
        .target(name: "AppleSession", dependencies: sourceTargets.map { .target(name: $0.name) } + ["RemindersStore", "AgendaGate", "SourceSupport"]),
        .executableTarget(name: "CalendarChecks", dependencies: ["CalendarSource"], path: "Tests/CalendarChecks"),
        .executableTarget(name: "CommandOriginChecks", dependencies: ["SourceSupport"], path: "Tests/CommandOriginChecks"),
        .executableTarget(name: "AppleBridgeHelper", dependencies: ["AppleSession"])
    ]
)
