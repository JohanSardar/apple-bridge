// swift-tools-version:6.3
import PackageDescription

let package = Package(
    name: "AppleTruth",
    platforms: [.macOS("26.0")],
    products: [.executable(name: "apple-truth", targets: ["AppleTruth"])],
    targets: [.executableTarget(name: "AppleTruth")]
)
