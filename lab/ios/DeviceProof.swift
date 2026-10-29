import Foundation

@MainActor
final class FakeDeviceContext: DeviceContextAdapter {
    var unavailable = false
    var denied = false
    var oversized = false
    func status() async throws -> DeviceStatus {
        DeviceStatus(batteryLevel: unavailable ? nil : 0.75, charging: unavailable ? "unknown" : "charging", wifiNetwork: unavailable ? nil : "[ab-test]", deviceName: "Bridge Test", focusFilterActive: unavailable ? nil : true)
    }
    func readNFC() async throws -> [NFCPayload] {
        if denied { throw CocoaError(.fileReadNoPermission) }
        return [NFCPayload(format: 1, type: Data([84]), identifier: Data(), payload: oversized ? Data(repeating: 1, count: 24000) : Data([2, 101, 110, 97]))]
    }
}

@main
struct DeviceProof {
    @MainActor static func main() async throws {
        let adapter = FakeDeviceContext()
        let service = DeviceContextService(adapter: adapter)
        let statusData = try await service.status()
        let status = try JSONDecoder().decode(DeviceStatus.self, from: statusData)
        if CommandLine.arguments.count > 1 { try statusData.write(to: URL(fileURLWithPath: CommandLine.arguments[1])) }
        precondition(status.batteryLevel == 0.75 && status.charging == "charging")
        precondition(status.wifiNetwork == "[ab-test]" && status.deviceName == "Bridge Test")
        precondition(status.focusFilterActive == true)
        let tags = try JSONDecoder().decode([NFCPayload].self, from: await service.readNFC())
        precondition(tags.count == 1 && tags[0].format == 1 && tags[0].payload == Data([2, 101, 110, 97]))
        adapter.unavailable = true
        let unknown = try JSONDecoder().decode(DeviceStatus.self, from: await service.status())
        precondition(unknown.batteryLevel == nil && unknown.wifiNetwork == nil && unknown.focusFilterActive == nil)
        precondition(unknown.charging == "unknown")
        adapter.denied = true
        do { _ = try await service.readNFC(); preconditionFailure() } catch {}
        precondition(tags[0].type == Data([84]) && tags[0].identifier.isEmpty)
        adapter.denied = false
        adapter.oversized = true
        do { _ = try await service.readNFC(); preconditionFailure() } catch {}
        print("PASS 8")
    }
}
