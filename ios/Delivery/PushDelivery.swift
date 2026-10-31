import CryptoKit
import Foundation
import SwiftUI
import UIKit
import UserNotifications

struct CommandPacket: Decodable {
    let ephemeral_key: Data
    let box: Data
    let signature: Data
}

struct PhoneCommand: Codable {
    let command_id: String
    let op: String
    let args: [String: JSONValue]
}

enum JSONValue: Codable {
    case string(String), number(Double), bool(Bool), object([String: JSONValue]), array([JSONValue]), null
    init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() { self = .null }
        else if let v = try? c.decode(String.self) { self = .string(v) }
        else if let v = try? c.decode(Bool.self) { self = .bool(v) }
        else if let v = try? c.decode(Double.self) { self = .number(v) }
        else if let v = try? c.decode([String: JSONValue].self) { self = .object(v) }
        else { self = .array(try c.decode([JSONValue].self)) }
    }
    func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .string(let v): try c.encode(v)
        case .number(let v): try c.encode(v)
        case .bool(let v): try c.encode(v)
        case .object(let v): try c.encode(v)
        case .array(let v): try c.encode(v)
        case .null: try c.encodeNil()
        }
    }
}

struct PhoneJob: Codable {
    let command: PhoneCommand
    let request_hash: String
    let expires: Int64
}

struct PhoneResult: Codable {
    let command_id: String
    let request_hash: String
    let state: String
    let data: JSONValue
}

struct Execution: Codable {
    let hash: String
    let expires: Int64
    var result: PhoneResult?
    var uploaded: Bool?
    var awaitingAction: Bool?
}

@MainActor
@Observable
final class PushDelivery {
    static let shared = PushDelivery()
    var lastResult = "None"
    var status = "Not registered" {
        didSet {
            #if DEBUG
            if ShellDeliveryQualification.active { return }
            #endif
            WidgetsCapability.status(status)
        }
    }
    var wakeCount = 0
    private var running = false
    private var drainAgain = false

    private var cacheURL: URL {
        get throws {
            let directory = try FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
            return directory.appendingPathComponent("command-receipts.json")
        }
    }

    private func save(_ cache: [String: Execution]) throws {
        try JSONEncoder().encode(cache).write(to: cacheURL, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
    }

    func register(token: String) async {
        do {
            var identity = try DeviceIdentity.load()
            identity.pushToken = token
            try identity.save()
            guard identity.deviceID != nil else { status = "Pairing required"; return }
            let payload = try JSONSerialization.data(withJSONObject: ["token": token])
            _ = try await PairingClient.signedRequest(identity: identity, path: "/v1/iphone/push-token", payload: payload)
            status = "Registered"
        } catch { status = "Registration pending" }
    }

    private func open(_ packet: CommandPacket, identity: DeviceIdentity) throws -> PhoneJob {
        guard let gateway = identity.gatewayKey, let id = identity.deviceID else { throw IdentityError.invalidCode }
        let message = Data("apple-bridge-iphone-command/1\0".utf8) + packet.ephemeral_key + packet.box
        guard try Curve25519.Signing.PublicKey(rawRepresentation: gateway).isValidSignature(packet.signature, for: message) else { throw IdentityError.requestFailed }
        let key = try Curve25519.KeyAgreement.PrivateKey(rawRepresentation: identity.encryption)
        let secret = try key.sharedSecretFromKeyAgreement(with: Curve25519.KeyAgreement.PublicKey(rawRepresentation: packet.ephemeral_key))
        let symmetric = secret.hkdfDerivedSymmetricKey(using: SHA256.self, salt: Data("apple-bridge-iphone/1".utf8), sharedInfo: Data(id.utf8), outputByteCount: 32)
        let plain = try AES.GCM.open(AES.GCM.SealedBox(combined: packet.box), using: symmetric)
        return try JSONDecoder().decode(PhoneJob.self, from: plain)
    }

    func drain() async {
        guard !RealQualification.active else { return }
        guard !running else { drainAgain = true; return }
        running = true
        defer {
            running = false
            if drainAgain {
                drainAgain = false
                Task { await drain() }
            }
        }
        do {
            var identity = try DeviceIdentity.load()
            guard identity.deviceID != nil else { status = "Pairing required"; return }
            if identity.gatewayKey == nil {
                let data = try await PairingClient.signedRequest(identity: identity, path: "/v1/iphone/status")
                identity.gatewayKey = try JSONDecoder().decode(PairedDevice.self, from: data).gateway_key
                try identity.save()
            }
            if let token = identity.pushToken, status != "Registered" { await register(token: token) }
            var cache: [String: Execution] = [:]
            if FileManager.default.fileExists(atPath: try cacheURL.path) {
                cache = try JSONDecoder().decode([String: Execution].self, from: Data(contentsOf: cacheURL))
            }
            cache = cache.filter { $0.value.uploaded != true || $0.value.expires + 86400 > Int64(Date().timeIntervalSince1970) }
            for result in try NotificationsCapability.responses() {
                guard cache[result.command_id]?.hash == result.request_hash else { continue }
                cache[result.command_id]?.result = result
                cache[result.command_id]?.uploaded = false
                cache[result.command_id]?.awaitingAction = false
                try save(cache)
                try NotificationsCapability.removeResponse(result.command_id)
            }
            for (id, execution) in cache where execution.uploaded != true {
                if let result = execution.result {
                    _ = try await PairingClient.signedRequest(identity: identity, path: "/v1/iphone/result", payload: JSONEncoder().encode(result))
                    cache[id]?.uploaded = true
                    try save(cache)
                }
            }
            let raw = try await PairingClient.signedRequest(identity: identity, path: "/v1/iphone/pending")
            let packets = try JSONDecoder().decode([CommandPacket].self, from: raw)
            for packet in packets {
                let job = try open(packet, identity: identity)
                guard job.expires > Int64(Date().timeIntervalSince1970), case .string(let target) = job.command.args["device_id"], target == identity.deviceID else { continue }
                let id = job.command.command_id
                if job.command.op == "bridge.approval" {
                    try await ApprovalsCapability.deliver(job)
                    continue
                }
                var result: PhoneResult
                if let prior = cache[id] {
                    guard prior.hash == job.request_hash else { throw IdentityError.requestFailed }
                    if prior.awaitingAction == true { continue }
                    result = prior.result ?? PhoneResult(command_id: id, request_hash: job.request_hash, state: "indeterminate", data: .null)
                } else {
                    cache[id] = Execution(hash: job.request_hash, expires: job.expires)
                    try save(cache)
                    if let handler = CommandHandlers.handler(for: job.command.op) {
                        do {
                            guard let output = try await handler(job) else {
                                cache[id]?.awaitingAction = true
                                try save(cache)
                                continue
                            }
                            result = PhoneResult(command_id: id, request_hash: job.request_hash, state: "observed", data: try JSONDecoder().decode(JSONValue.self, from: output))
                        } catch {
                            result = PhoneResult(command_id: id, request_hash: job.request_hash, state: "indeterminate", data: .null)
                        }
                    } else {
                        result = PhoneResult(command_id: id, request_hash: job.request_hash, state: "rejected", data: .null)
                    }
                    cache[id]?.result = result
                    try save(cache)
                }
                cache[id]?.result = result
                cache[id]?.uploaded = false
                try save(cache)
                _ = try await PairingClient.signedRequest(identity: identity, path: "/v1/iphone/result", payload: JSONEncoder().encode(result))
                cache[id]?.uploaded = true
                try save(cache)
                lastResult = result.state
            }
        } catch { status = "Delivery pending" }
    }
}

final class BridgeAppDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
    func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        #if DEBUG
        let qualifying = ShellDeliveryQualification.active || RealQualification.active
        #else
        let qualifying = false
        #endif
        if !qualifying {
            Task {
                #if DEBUG
                try? await ShellDeliveryQualification.restoreBackup()
                #endif
                LocationCapability.resume()
                WidgetsCapability.resume()
                await HealthReadController.shared.resume()
            }
        }
        UNUserNotificationCenter.current().delegate = self
        PushDelivery.shared.status = "Registering"
        #if DEBUG
        if !RealQualification.active && !ProcessInfo.processInfo.arguments.contains("--ab-real-restore") { application.registerForRemoteNotifications() }
        #else
        application.registerForRemoteNotifications()
        #endif
        return true
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification, withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
        completionHandler([.banner, .sound, .list])
    }

    nonisolated func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        let info = response.notification.request.content.userInfo
        if info["approval"] as? Bool == true, let id = info["command_id"] as? String, let hash = info["request_hash"] as? String, let expires = info["expires"] as? Int64 {
            do {
                try await ApprovalsCapability.present(id: id, hash: hash, action: response.actionIdentifier, expires: expires)
            } catch {
                await MainActor.run { PushDelivery.shared.status = "Approval pending" }
            }
            return
        }
        guard let id = info["command_id"] as? String, let hash = info["request_hash"] as? String, let buttons = info["buttons"] as? [String] else { return }
        let action = response.actionIdentifier
        do {
            try await NotificationsCapability.record(id: id, hash: hash, buttons: buttons, action: action)
            await PushDelivery.shared.drain()
        } catch {
            await MainActor.run { PushDelivery.shared.status = "Action delivery pending" }
        }
    }

    func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        let token = deviceToken.map { String(format: "%02x", $0) }.joined()
        Task { await PushDelivery.shared.register(token: token) }
    }

    func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: Error) {
        PushDelivery.shared.status = "APNs registration unavailable"
    }

    func application(_ application: UIApplication, didReceiveRemoteNotification userInfo: [AnyHashable: Any], fetchCompletionHandler completionHandler: @escaping (UIBackgroundFetchResult) -> Void) {
        PushDelivery.shared.wakeCount += 1
        Task {
            await PushDelivery.shared.drain()
            completionHandler(.newData)
        }
    }
}
