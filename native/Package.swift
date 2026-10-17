// swift-tools-version:6.3
import PackageDescription

let package = Package(
    name: "AppleBridgeHelper",
    platforms: [.macOS("26.0")],
    products: [.executable(name: "AppleBridgeHelper", targets: ["AppleBridgeHelper"])],
    targets: [
        .target(name: "MessagesSource"),
        .target(name: "AppleSession", dependencies: ["MessagesSource"]),
        .executableTarget(name: "AppleBridgeHelper", dependencies: ["AppleSession"])
    ]
)
