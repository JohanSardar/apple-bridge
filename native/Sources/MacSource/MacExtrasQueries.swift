import Foundation
import SourceSupport
import CoreAudio

public enum MacExtrasQueries {
    public static func run(_ op: String, args: JSON) async throws -> JSON {
        if op == "mac.recognize_song" {
            guard let bytes = Data(base64Encoded: args["audio_blob"].string), !bytes.isEmpty, bytes.count <= 8 * 1024 * 1024 else { throw SourceFailure.invalid }
            return .object(["limited": .bool(true), "reason": .string("ShazamKit entitlement and fixture qualification required")])
        }
        guard args == .object([:]) else { throw SourceFailure.invalid }
        if op == "mac.time_machine_status" {
            return try await Task.detached { () throws -> JSON in
                let process = Process()
                process.executableURL = URL(fileURLWithPath: "/usr/bin/tmutil")
                process.arguments = ["status"]
                let pipe = Pipe()
                process.standardOutput = pipe
                process.standardError = FileHandle.nullDevice
                try process.run()
                let data = pipe.fileHandleForReading.readDataToEndOfFile()
                process.waitUntilExit()
                guard process.terminationStatus == 0, let text = String(data: data, encoding: .utf8) else { throw SourceFailure.unavailable }
                return .object(["status": .string(text)])
            }.value
        }
        if op == "mac.bluetooth_devices" {
            return try await Task.detached { () throws -> JSON in
                let process = Process()
                process.executableURL = URL(fileURLWithPath: "/usr/sbin/system_profiler")
                process.arguments = ["SPBluetoothDataType", "-json", "-timeout", "15"]
                let pipe = Pipe()
                process.standardOutput = pipe
                process.standardError = FileHandle.nullDevice
                try process.run()
                let data = pipe.fileHandleForReading.readDataToEndOfFile()
                process.waitUntilExit()
                guard process.terminationStatus == 0,
                      let report = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                      let controllers = report["SPBluetoothDataType"] as? [[String: Any]] else { throw SourceFailure.unavailable }
                var devices: [JSON] = []
                for controller in controllers {
                    for (key, connected) in [("device_connected", true), ("device_not_connected", false)] {
                        for group in controller[key] as? [[String: Any]] ?? [] {
                            for (name, _) in group {
                                devices.append(.object(["name": .string(name), "connected": .bool(connected)]))
                            }
                        }
                    }
                }
                return .object(["devices": .array(devices)])
            }.value
        }
        if op == "mac.airplay_routes" {
            var address = AudioObjectPropertyAddress(mSelector: kAudioHardwarePropertyDevices, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
            var size: UInt32 = 0
            guard AudioObjectGetPropertyDataSize(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size) == noErr else { throw SourceFailure.unavailable }
            var devices = [AudioDeviceID](repeating: 0, count: Int(size) / MemoryLayout<AudioDeviceID>.size)
            guard AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &devices) == noErr else { throw SourceFailure.unavailable }
            var routes: [JSON] = []
            for device in devices {
                var transport: UInt32 = 0
                size = 4
                address.mSelector = kAudioDevicePropertyTransportType
                guard AudioObjectGetPropertyData(device, &address, 0, nil, &size, &transport) == noErr else { continue }
                if transport == kAudioDeviceTransportTypeAirPlay {
                    routes.append(.object(["device": .string(String(device))]))
                }
            }
            var current: AudioDeviceID = 0
            size = UInt32(MemoryLayout<AudioDeviceID>.size)
            address.mSelector = kAudioHardwarePropertyDefaultOutputDevice
            guard AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &current) == noErr else { throw SourceFailure.unavailable }
            return .object(["routes": .array(routes), "current_device": .string(String(current))])
        }
        throw SourceFailure.unsupported
    }
}
