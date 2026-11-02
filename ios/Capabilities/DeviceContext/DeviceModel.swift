import Foundation

struct DeviceStatus: Codable {
    let batteryLevel: Double?
    let charging: String
    let wifiNetwork: String?
    let deviceName: String
    let focusFilterActive: Bool?
}

@MainActor
protocol DeviceContextAdapter {
    func status() async throws -> DeviceStatus
}
