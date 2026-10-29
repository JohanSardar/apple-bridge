import Foundation

@MainActor
final class FakeExtras: ExtrasAdapter {
    var calls = 0
    func weather(latitude: Double, longitude: Double) async throws -> Data { calls += 1; return Data("{\"temperature_celsius\":12}".utf8) }
    func journal() async throws -> Data { calls += 1; return Data("{\"title\":\"Bridge Test\"}".utf8) }
    func energy(venue: UUID) async throws -> Data { calls += 1; return Data("{\"rating\":0.8}".utf8) }
}

@main
struct ExtrasChecks {
    @MainActor
    static func main() async throws {
        let adapter = FakeExtras()
        let requests = [
            ("iphone.weather", "{\"device_id\":\"iphone:test\",\"latitude\":10,\"longitude\":20}", "temperature_celsius"),
            ("iphone.journal.suggestions", "{\"device_id\":\"iphone:test\"}", "title"),
            ("iphone.energy.hints", "{\"device_id\":\"iphone:test\",\"venue_id\":\"00000000-0000-0000-0000-000000000001\"}", "rating"),
        ]
        for (name, json, key) in requests {
            let raw = Data(json.utf8)
            for (enabled, granted) in [(false, true), (true, false)] {
                let before = adapter.calls
                do {
                    _ = try await ExtrasService.execute(name, raw: raw, enabled: enabled, granted: granted, adapter: adapter)
                    fatalError("gate accepted")
                } catch ExtrasError.disabled {} catch ExtrasError.unavailable {}
                precondition(adapter.calls == before)
            }
            let result = try await ExtrasService.execute(name, raw: raw, enabled: true, granted: true, adapter: adapter)
            let object = try JSONSerialization.jsonObject(with: result) as! [String: Any]
            precondition(object[key] != nil)
        }
        do {
            _ = try await ExtrasService.execute("iphone.weather", raw: Data("{\"device_id\":\"iphone:test\",\"latitude\":91,\"longitude\":0}".utf8), enabled: true, granted: true, adapter: adapter)
            fatalError("invalid coordinates accepted")
        } catch ExtrasError.invalidArguments {}
        precondition(adapter.calls == 3)
        print("PASS")
    }
}
