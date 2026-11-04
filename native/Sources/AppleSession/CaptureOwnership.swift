import Foundation

struct CaptureOwnership: Sendable {
    static let tabsOnly: Set<String> = ["safari"]

    let rust: Set<String>

    init(rust: Set<String>) { self.rust = rust }

    init(home: URL) {
        guard let data = try? Data(contentsOf: home.appendingPathComponent("session/rust-capture.json")),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let sources = object["sources"] as? [String: Any] else { self.init(rust: []); return }
        self.init(rust: Set(sources.keys))
    }

    func swiftCaptures(_ source: String) -> Bool {
        !rust.contains(source) || Self.tabsOnly.contains(source)
    }
}
