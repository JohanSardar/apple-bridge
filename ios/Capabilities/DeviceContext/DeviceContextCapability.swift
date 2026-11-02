import AppIntents
import CoreLocation
import Foundation
import NetworkExtension
import UIKit

struct BridgeFocusFilter: SetFocusFilterIntent {
    static let title: LocalizedStringResource = "Apple Bridge context"
    @Parameter(title: "Active", default: false) var active: Bool
    var displayRepresentation: DisplayRepresentation { DisplayRepresentation(title: "Apple Bridge context") }
    func perform() async throws -> some IntentResult { .result() }
}

@MainActor
final class SystemDeviceContext: DeviceContextAdapter {
    private static let location = CLLocationManager()
    func status() async throws -> DeviceStatus {
        let device = UIDevice.current
        device.isBatteryMonitoringEnabled = true
        let charging: String
        switch device.batteryState {
        case .charging: charging = "charging"
        case .full: charging = "full"
        case .unplugged: charging = "unplugged"
        default: charging = "unknown"
        }
        if Self.location.authorizationStatus == .notDetermined {
            Self.location.requestWhenInUseAuthorization()
        }
        let network = await NEHotspotNetwork.fetchCurrent()
        let filter: Bool?
        do { filter = try await BridgeFocusFilter.current.active }
        catch SetFocusFilterIntentError.notFound { filter = false }
        catch { filter = nil }
        return DeviceStatus(batteryLevel: device.batteryLevel < 0 ? nil : Double(device.batteryLevel), charging: charging, wifiNetwork: network?.ssid, deviceName: device.name, focusFilterActive: filter)
    }
}

@MainActor
enum DeviceContextCapability {
    static let descriptor = Capability(id: "device-context", title: "Device context", permission: "Wi-Fi requires Access Wi-Fi Information and location authorization", available: true, handlers: [
        "iphone.device.status": { _ in try JSONEncoder().encode(await SystemDeviceContext().status()) },
    ])
}
