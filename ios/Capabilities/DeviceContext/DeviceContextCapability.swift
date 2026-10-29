import AppIntents
import CoreLocation
import CoreNFC
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

    func readNFC() async throws -> [NFCPayload] {
        guard UIApplication.shared.applicationState == .active else { throw CocoaError(.featureUnsupported) }
        return try await NFCReader.shared.read()
    }
}

@MainActor
final class NFCReader: NSObject, @preconcurrency NFCNDEFReaderSessionDelegate {
    static let shared = NFCReader()
    private var session: NFCNDEFReaderSession?
    private var continuation: CheckedContinuation<[NFCPayload], Error>?

    func read() async throws -> [NFCPayload] {
        guard NFCNDEFReaderSession.readingAvailable, session == nil else { throw CocoaError(.featureUnsupported) }
        return try await withCheckedThrowingContinuation { continuation in
            self.continuation = continuation
            let session = NFCNDEFReaderSession(delegate: self, queue: .main, invalidateAfterFirstRead: true)
            self.session = session
            session.alertMessage = "Hold your iPhone near the automation tag."
            session.begin()
        }
    }

    func readerSession(_ session: NFCNDEFReaderSession, didInvalidateWithError error: Error) {
            guard self.session === session else { return }
            let pending = continuation
            continuation = nil
            self.session = nil
            pending?.resume(throwing: error)
    }

    func readerSession(_ session: NFCNDEFReaderSession, didDetectNDEFs messages: [NFCNDEFMessage]) {
        let records = messages.flatMap(\.records).map { NFCPayload(format: $0.typeNameFormat.rawValue, type: $0.type, identifier: $0.identifier, payload: $0.payload) }
            guard self.session === session else { return }
            let pending = continuation
            continuation = nil
            session.invalidate()
            self.session = nil
            pending?.resume(returning: records)
    }
}

@MainActor
enum DeviceContextCapability {
    static let descriptor = Capability(id: "device-context", title: "Device context and NFC", permission: "Wi-Fi requires Access Wi-Fi Information and location authorization; NFC requires foreground", available: true, handlers: [
        "iphone.device.status": { _ in try await DeviceContextService(adapter: SystemDeviceContext()).status() },
        "iphone.nfc.read": { _ in try await DeviceContextService(adapter: SystemDeviceContext()).readNFC() },
    ])
}
