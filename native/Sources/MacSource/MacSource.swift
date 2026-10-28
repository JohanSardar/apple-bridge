import Foundation
import AppKit
import CoreAudio
import CoreWLAN
import CoreLocation
import Network
import IOKit.ps
import IOKit.pwr_mgt
import SourceSupport

@MainActor
public final class MacSource: Source {
    public nonisolated let name = "mac"
    private var generation = UUID().uuidString
    private var path: NWPath?
    private var committed: [String: JSON] = [:]
    public init() {}
    public func open() async -> SourceReadiness {
        SourceReadiness(state: "ready", coverage: .object(["fields": .string("partial"), "missing": .array([.string("Night Shift has no supported public API"), .string("SSID requires Location authorization")])]))
    }
    private func power() -> JSON {
        var value: [String: JSON] = ["source": .string("unknown"), "charging": .bool(false), "low_power_mode": .bool(ProcessInfo.processInfo.isLowPowerModeEnabled), "keep_awake_held": .bool(false)]
        if let info = IOPSCopyPowerSourcesInfo()?.takeRetainedValue() {
            value["source"] = .string(IOPSGetProvidingPowerSourceType(info)?.takeUnretainedValue() as String? ?? "unknown")
            for source in IOPSCopyPowerSourcesList(info)?.takeRetainedValue() as? [CFTypeRef] ?? [] {
                if let d = IOPSGetPowerSourceDescription(info, source)?.takeUnretainedValue() as? [String: Any] {
                    if let capacity = d[kIOPSCurrentCapacityKey] as? Double, let max = d[kIOPSMaxCapacityKey] as? Double, max > 0 { value["battery_percent"] = .number(100 * capacity / max) }
                    value["charging"] = .bool(d[kIOPSIsChargingKey] as? Bool ?? false)
                }
            }
        }
        let remaining = IOPSGetTimeRemainingEstimate()
        if remaining >= 0 { value["time_remaining"] = .number(remaining) }
        var assertions: Unmanaged<CFDictionary>?
        if IOPMCopyAssertionsStatus(&assertions) == kIOReturnSuccess, let states = assertions?.takeRetainedValue() as? [String: Int] {
            value["keep_awake_held"] = .bool((states[kIOPMAssertionTypePreventUserIdleSystemSleep] ?? 0) > 0)
        }
        return .object(value)
    }
    private func audio() throws -> [(String, JSON)] {
        var address = AudioObjectPropertyAddress(mSelector: kAudioHardwarePropertyDevices, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
        var size: UInt32 = 0
        guard AudioObjectGetPropertyDataSize(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size) == noErr else { throw SourceFailure.unavailable }
        var devices = [AudioDeviceID](repeating: 0, count: Int(size) / MemoryLayout<AudioDeviceID>.size)
        guard AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &devices) == noErr else { throw SourceFailure.unavailable }
        func defaultDevice(_ selector: AudioObjectPropertySelector) -> AudioDeviceID {
            var a = AudioObjectPropertyAddress(mSelector: selector, mScope: kAudioObjectPropertyScopeGlobal, mElement: 0)
            var device: AudioDeviceID = 0; var count = UInt32(MemoryLayout.size(ofValue: device))
            _ = AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &a, 0, nil, &count, &device)
            return device
        }
        let input = defaultDevice(kAudioHardwarePropertyDefaultInputDevice), output = defaultDevice(kAudioHardwarePropertyDefaultOutputDevice)
        return devices.map { device in
            var a = AudioObjectPropertyAddress(mSelector: kAudioObjectPropertyName, mScope: kAudioObjectPropertyScopeGlobal, mElement: 0)
            var title: Unmanaged<CFString>?; var count = UInt32(MemoryLayout<Unmanaged<CFString>?>.size)
            _ = AudioObjectGetPropertyData(device, &a, 0, nil, &count, &title)
            func streams(_ scope: AudioObjectPropertyScope) -> Bool {
                var a = AudioObjectPropertyAddress(mSelector: kAudioDevicePropertyStreams, mScope: scope, mElement: 0); var count: UInt32 = 0
                return AudioObjectGetPropertyDataSize(device, &a, 0, nil, &count) == noErr && count > 0
            }
            var value: [String: JSON] = ["name": .string(title?.takeRetainedValue() as String? ?? "Unknown device"), "input": .bool(streams(kAudioDevicePropertyScopeInput)), "output": .bool(streams(kAudioDevicePropertyScopeOutput)), "default": .bool(device == input || device == output)]
            let scope = streams(kAudioDevicePropertyScopeOutput) ? kAudioDevicePropertyScopeOutput : kAudioDevicePropertyScopeInput
            a = AudioObjectPropertyAddress(mSelector: kAudioDevicePropertyVolumeScalar, mScope: scope, mElement: 0)
            var volume: Float32 = 0; count = 4
            if AudioObjectGetPropertyData(device, &a, 0, nil, &count, &volume) == noErr { value["volume"] = .number(Double(volume)) }
            a.mSelector = kAudioDevicePropertyMute; var muted: UInt32 = 0; count = 4
            if AudioObjectGetPropertyData(device, &a, 0, nil, &count, &muted) == noErr { value["muted"] = .bool(muted != 0) }
            return (String(device), .object(value))
        }
    }
    private func scan(_ sink: any ObservationSink, live: Bool) async throws {
        var current: [String: JSON] = [:]
        for (key, value) in try audio() { current["mac.audio_device/" + key] = value }
        current["mac.power/system"] = power()
        current["mac.display/system"] = .object(["appearance": .string(UserDefaults.standard.string(forKey: "AppleInterfaceStyle") == "Dark" ? "dark" : "light"), "displays": .array(NSScreen.screens.map { .object(["name": .string($0.localizedName), "width": .number($0.frame.width), "height": .number($0.frame.height)]) })])
        var network: [String: JSON] = ["connected": .bool(path?.status == .satisfied), "vpn": .bool(path?.availableInterfaces.contains(where: { $0.name.hasPrefix("utun") || $0.name.hasPrefix("ppp") }) ?? false)]
        if let wifi = CWWiFiClient.shared().interface() {
            network["wifi_power"] = .bool(wifi.powerOn())
            let authorization = CLLocationManager().authorizationStatus
            if authorization == .authorizedAlways, let ssid = wifi.ssid() { network["ssid"] = .string(ssid) }
        }
        if let interface = path?.availableInterfaces.first { network["interface"] = .string(interface.name) }
        current["mac.network/system"] = .object(network)
        for app in NSWorkspace.shared.runningApplications {
            guard let bundle = app.bundleIdentifier else { continue }
            current["mac.app/" + String(app.processIdentifier)] = .object(["bundle_id": .string(bundle), "name": .string(app.localizedName ?? bundle), "running": .bool(!app.isTerminated), "frontmost": .bool(app.isActive), "hidden": .bool(app.isHidden)])
        }
        let started = timestamp()
        var observations: [JSON] = []
        for (key, value) in current where !live || committed[key] != value {
            let parts = key.split(separator: "/", maxSplits: 1).map(String.init)
            observations.append(.object(["kind": .string("upsert"), "collection": .string(parts[0]), "native_key": .string(parts[1]), "scoped_alias": .string("mac/" + key), "data": value, "observed_at": .string(started)]))
        }
        for (key, value) in committed where current[key] == nil {
            let parts = key.split(separator: "/", maxSplits: 1).map(String.init)
            observations.append(.object(["kind": .string("absent"), "collection": .string(parts[0]), "native_key": .string(parts[1]), "data": value, "revalidated": .bool(true), "observed_at": .string(started)]))
        }
        try await sink.emitCheckpoint(source: name, generation: generation, checkpoint: UUID().uuidString, observations: observations, scanID: UUID().uuidString, scanStartedAt: started, scope: ["mac.audio_device", "mac.display", "mac.power", "mac.network", "mac.app"], completionEvidence: "complete", blobHandles: [:], live: live)
        committed = current
    }
    public func capture(_ resume: JSON, into sink: any ObservationSink) async throws {
        if !resume["generation"].string.isEmpty { generation = resume["generation"].string }
        let initialDevices = Set(try audio().compactMap { UInt32($0.0) } + [AudioObjectID(kAudioObjectSystemObject)])
        let (changes, continuation) = AsyncStream<Void>.makeStream(bufferingPolicy: .bufferingNewest(1))
        let wifi = CWWiFiClient.shared()
        let wifiEvents = MacWiFiChanges(continuation)
        wifi.delegate = wifiEvents
        try? wifi.startMonitoringEvent(with: .powerDidChange)
        try? wifi.startMonitoringEvent(with: .ssidDidChange)
        try? wifi.startMonitoringEvent(with: .linkDidChange)
        let monitor = NWPathMonitor()
        monitor.pathUpdateHandler = { path in Task { @MainActor in self.path = path; continuation.yield(()) } }
        monitor.start(queue: DispatchQueue(label: "mac.network"))
        let center = NSWorkspace.shared.notificationCenter
        let tokens = [NSWorkspace.didLaunchApplicationNotification, NSWorkspace.didTerminateApplicationNotification, NSWorkspace.didActivateApplicationNotification, NSWorkspace.didHideApplicationNotification, NSWorkspace.didUnhideApplicationNotification, NSWorkspace.didWakeNotification].map { name in
            center.addObserver(forName: name, object: nil, queue: .main) { _ in continuation.yield(()) }
        }
        let lowPower = NotificationCenter.default.addObserver(forName: .NSProcessInfoPowerStateDidChange, object: nil, queue: .main) { _ in continuation.yield(()) }
        let appearance = DistributedNotificationCenter.default().addObserver(forName: NSNotification.Name("AppleInterfaceThemeChangedNotification"), object: nil, queue: .main) { _ in continuation.yield(()) }
        let display = NotificationCenter.default.addObserver(forName: NSApplication.didChangeScreenParametersNotification, object: nil, queue: .main) { _ in continuation.yield(()) }
        let powerContext = MacChange(continuation)
        let power = IOPSNotificationCreateRunLoopSource({ context in
            guard let context else { return }
            Unmanaged<MacChange>.fromOpaque(context).takeUnretainedValue().signal()
        }, Unmanaged.passUnretained(powerContext).toOpaque())?.takeRetainedValue()
        if let power { CFRunLoopAddSource(CFRunLoopGetMain(), power, .commonModes) }
        var address = AudioObjectPropertyAddress(mSelector: kAudioObjectPropertySelectorWildcard, mScope: kAudioObjectPropertyScopeWildcard, mElement: kAudioObjectPropertyElementWildcard)
        let listener: AudioObjectPropertyListenerBlock = { _, _ in continuation.yield(()) }
        var devices = initialDevices
        for device in devices { AudioObjectAddPropertyListenerBlock(device, &address, .main, listener) }
        defer {
            try? wifi.stopMonitoringAllEvents(); wifi.delegate = nil
            withExtendedLifetime(wifiEvents) {}
            monitor.cancel(); tokens.forEach { center.removeObserver($0) }; NotificationCenter.default.removeObserver(display); NotificationCenter.default.removeObserver(lowPower); DistributedNotificationCenter.default().removeObserver(appearance)
            if let power { CFRunLoopRemoveSource(CFRunLoopGetMain(), power, .commonModes) }
            for device in devices { AudioObjectRemovePropertyListenerBlock(device, &address, .main, listener) }
            withExtendedLifetime(powerContext) {}
            continuation.finish()
        }
        try await scan(sink, live: false)
        for await _ in changes {
            try Task.checkCancellation()
            let current = Set(try audio().compactMap { UInt32($0.0) } + [AudioObjectID(kAudioObjectSystemObject)])
            for device in devices.subtracting(current) { AudioObjectRemovePropertyListenerBlock(device, &address, .main, listener) }
            for device in current.subtracting(devices) { AudioObjectAddPropertyListenerBlock(device, &address, .main, listener) }
            devices = current
            try await scan(sink, live: true)
        }
    }
    public func prepare(_ command: JSON) async throws -> Prepared { throw SourceFailure.unsupported }
    public func dispatch(_ prepared: Prepared) async throws -> JSON { throw SourceFailure.unsupported }
    public func resolve(_ command: String, evidence: JSON) async -> JSON { .null }
}

private final class MacChange: @unchecked Sendable {
    let continuation: AsyncStream<Void>.Continuation
    init(_ continuation: AsyncStream<Void>.Continuation) { self.continuation = continuation }
    func signal() { continuation.yield(()) }
}

private final class MacWiFiChanges: NSObject, CWEventDelegate, @unchecked Sendable {
    let continuation: AsyncStream<Void>.Continuation
    init(_ continuation: AsyncStream<Void>.Continuation) { self.continuation = continuation }
    func powerStateDidChangeForWiFiInterface(withName interfaceName: String) { continuation.yield(()) }
    func ssidDidChangeForWiFiInterface(withName interfaceName: String) { continuation.yield(()) }
    func linkDidChangeForWiFiInterface(withName interfaceName: String) { continuation.yield(()) }
}
