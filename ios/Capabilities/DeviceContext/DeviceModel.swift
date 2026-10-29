import Foundation

struct DeviceStatus: Codable {
    let batteryLevel: Double?
    let charging: String
    let wifiNetwork: String?
    let deviceName: String
    let focusFilterActive: Bool?
}

struct NFCPayload: Codable {
    let format: UInt8
    let type: Data
    let identifier: Data
    let payload: Data
}

@MainActor
protocol DeviceContextAdapter {
    func status() async throws -> DeviceStatus
    func readNFC() async throws -> [NFCPayload]
}

@MainActor
struct DeviceContextService {
    let adapter: any DeviceContextAdapter
    func status() async throws -> Data { try JSONEncoder().encode(await adapter.status()) }
    func readNFC() async throws -> Data {
        let payload = try JSONEncoder().encode(await adapter.readNFC())
        guard payload.count <= 20000 else { throw CocoaError(.coderInvalidValue) }
        return payload
    }
}
