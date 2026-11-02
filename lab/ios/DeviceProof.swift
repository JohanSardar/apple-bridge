import Foundation

@MainActor
final class FakeDeviceContext: DeviceContextAdapter {
    var unavailable = false
    func status() async throws -> DeviceStatus {
        DeviceStatus(batteryLevel: unavailable ? nil : 0.75, charging: unavailable ? "unknown" : "charging", wifiNetwork: unavailable ? nil : "[ab-test]", deviceName: "Bridge Test", focusFilterActive: unavailable ? nil : true)
    }
}

@main
struct DeviceProof {
    @MainActor static func main() async throws {
        let adapter = FakeDeviceContext()
        let statusData = try JSONEncoder().encode(await adapter.status())
        let status = try JSONDecoder().decode(DeviceStatus.self, from: statusData)
        if CommandLine.arguments.count > 1 { try statusData.write(to: URL(fileURLWithPath: CommandLine.arguments[1])) }
        precondition(status.batteryLevel == 0.75 && status.charging == "charging")
        precondition(status.wifiNetwork == "[ab-test]" && status.deviceName == "Bridge Test")
        precondition(status.focusFilterActive == true)
        adapter.unavailable = true
        let unknown = try JSONDecoder().decode(DeviceStatus.self, from: JSONEncoder().encode(await adapter.status()))
        precondition(unknown.batteryLevel == nil && unknown.wifiNetwork == nil && unknown.focusFilterActive == nil)
        precondition(unknown.charging == "unknown")
        print("PASS 5")
    }
}
