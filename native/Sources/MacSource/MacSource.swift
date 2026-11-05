import Foundation
import UserNotifications
import Carbon
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
    private var assertion: IOPMAssertionID = 0
    private var expiry: Task<Void, Never>?
    private var path: NWPath?
    public init() {}
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
    public func state() throws -> [String: JSON] {
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
        return current
    }
    public func observe(_ handler: @MainActor (Bool) async throws -> Void) async throws {
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
        func addresses(_ device: AudioObjectID) -> [AudioObjectPropertyAddress] {
            if device == AudioObjectID(kAudioObjectSystemObject) {
                return [kAudioHardwarePropertyDevices, kAudioHardwarePropertyDefaultInputDevice, kAudioHardwarePropertyDefaultOutputDevice].map { AudioObjectPropertyAddress(mSelector: $0, mScope: kAudioObjectPropertyScopeGlobal, mElement: 0) }
            }
            var result = [AudioObjectPropertyAddress(mSelector: kAudioObjectPropertyName, mScope: kAudioObjectPropertyScopeGlobal, mElement: 0)]
            for scope in [kAudioDevicePropertyScopeInput, kAudioDevicePropertyScopeOutput] {
                for selector in [kAudioDevicePropertyStreams, kAudioDevicePropertyVolumeScalar, kAudioDevicePropertyMute] {
                    result.append(AudioObjectPropertyAddress(mSelector: selector, mScope: scope, mElement: 0))
                }
            }
            return result
        }
        let listener: AudioObjectPropertyListenerBlock = { _, _ in continuation.yield(()) }
        var devices = initialDevices
        for device in devices { for var address in addresses(device) { AudioObjectAddPropertyListenerBlock(device, &address, .main, listener) } }
        defer {
            try? wifi.stopMonitoringAllEvents(); wifi.delegate = nil
            withExtendedLifetime(wifiEvents) {}
            monitor.cancel(); tokens.forEach { center.removeObserver($0) }; NotificationCenter.default.removeObserver(display); NotificationCenter.default.removeObserver(lowPower); DistributedNotificationCenter.default().removeObserver(appearance)
            if let power { CFRunLoopRemoveSource(CFRunLoopGetMain(), power, .commonModes) }
            for device in devices { for var address in addresses(device) { AudioObjectRemovePropertyListenerBlock(device, &address, .main, listener) } }
            withExtendedLifetime(powerContext) {}
            continuation.finish()
        }
        try await handler(false)
        for await _ in changes {
            try Task.checkCancellation()
            let current = Set(try audio().compactMap { UInt32($0.0) } + [AudioObjectID(kAudioObjectSystemObject)])
            for device in devices.subtracting(current) { for var address in addresses(device) { AudioObjectRemovePropertyListenerBlock(device, &address, .main, listener) } }
            for device in current.subtracting(devices) { for var address in addresses(device) { AudioObjectAddPropertyListenerBlock(device, &address, .main, listener) } }
            devices = current
            try await handler(true)
        }
    }
    public func primitive(_ operation: String, args: JSON) async throws -> JSON {
        switch operation {
        case "mac.script":
            return try await AppleScriptRunner.run(args["source"].string, target: args["target"].string.isEmpty ? nil : args["target"].string)
        case "mac.pasteboard_policy":
            return .object(["always_allow": .bool(NSPasteboard.general.accessBehavior == .alwaysAllow)])
        case "mac.clipboard_write":
            let board = NSPasteboard.general
            guard board.accessBehavior == .alwaysAllow else { throw SourceFailure.permissionMissing }
            let text = args["text"].string
            let item = NSPasteboardItem()
            guard item.setString(text, forType: .string), item.setData(Data(), forType: NSPasteboard.PasteboardType("org.nspasteboard.TransientType")), item.setData(Data(), forType: NSPasteboard.PasteboardType("org.nspasteboard.ConcealedType")) else { throw SourceFailure.unavailable }
            board.clearContents()
            return .object(["matches": .bool(board.writeObjects([item]) && board.string(forType: .string) == text)])
        case "mac.notification_post":
            let center = UNUserNotificationCenter.current()
            let settings = await center.notificationSettings()
            guard settings.authorizationStatus == .authorized || settings.authorizationStatus == .provisional else { throw SourceFailure.permissionMissing }
            let content = UNMutableNotificationContent()
            content.title = args["title"].string
            content.body = args["body"].string
            if !args["action_url"].string.isEmpty { content.userInfo = ["action_url": args["action_url"].string] }
            try await center.add(UNNotificationRequest(identifier: args["identifier"].string, content: content, trigger: nil))
            return .object([:])
        case "mac.notification_delivered":
            let identifier = args["identifier"].string
            return .object(["delivered": .bool(await UNUserNotificationCenter.current().deliveredNotifications().contains { $0.request.identifier == identifier })])
        case "mac.open_url":
            guard let url = URL(string: args["url"].string), NSWorkspace.shared.open(url) else { throw SourceFailure.unavailable }
            return .object([:])
        case "mac.frontmost":
            return .object(["bundle_id": .string(NSWorkspace.shared.frontmostApplication?.bundleIdentifier ?? "")])
        case "mac.set_wallpaper":
            let bytes = try Data(contentsOf: URL(fileURLWithPath: args["path"].string))
            guard let bitmap = NSBitmapImageRep(data: bytes), let png = bitmap.representation(using: .png, properties: [:]), !NSScreen.screens.isEmpty else { throw SourceFailure.invalid }
            let directory = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("AppleBridge/wallpapers")
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            let url = directory.appendingPathComponent(UUID().uuidString + ".png")
            try png.write(to: url, options: .atomic)
            for screen in NSScreen.screens { try NSWorkspace.shared.setDesktopImageURL(url, for: screen, options: [:]) }
            return .object(["matches": .bool(NSScreen.screens.allSatisfy { NSWorkspace.shared.desktopImageURL(for: $0) == url })])
        case "mac.app_installed":
            return .object(["installed": .bool(NSWorkspace.shared.urlForApplication(withBundleIdentifier: args["bundle_id"].string) != nil)])
        case "mac.app_launch":
            guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: args["bundle_id"].string) else { throw SourceFailure.invalid }
            _ = try await NSWorkspace.shared.openApplication(at: url, configuration: NSWorkspace.OpenConfiguration())
            return .object([:])
        case "mac.app_activate", "mac.app_quit":
            guard let app = NSRunningApplication.runningApplications(withBundleIdentifier: args["bundle_id"].string).first else { throw SourceFailure.conflict }
            guard operation == "mac.app_quit" ? app.terminate() : app.activate() else { throw SourceFailure.unavailable }
            return .object([:])
        case "mac.app_state":
            let apps = NSRunningApplication.runningApplications(withBundleIdentifier: args["bundle_id"].string)
            return .object(["running": .bool(!apps.isEmpty), "active": .bool(apps.contains { $0.isActive })])
        case "mac.keep_awake":
            expiry?.cancel(); expiry = nil
            if assertion != 0 { IOPMAssertionRelease(assertion); assertion = 0 }
            if args["on"].bool {
                guard IOPMAssertionCreateWithName(kIOPMAssertionTypePreventUserIdleSystemSleep as CFString, IOPMAssertionLevel(kIOPMAssertionLevelOn), "Apple Bridge keep awake" as CFString, &assertion) == kIOReturnSuccess else { throw SourceFailure.unavailable }
                let duration = args["duration"].number
                if duration > 0 {
                    let held = assertion
                    expiry = Task { [weak self] in
                        do { try await Task.sleep(for: .seconds(duration)) } catch { return }
                        guard !Task.isCancelled, let self, self.assertion == held else { return }
                        if self.assertion != 0 { IOPMAssertionRelease(self.assertion); self.assertion = 0 }
                    }
                }
            }
            return .object(["held": .bool(assertion != 0 && IOPMAssertionCopyProperties(assertion) != nil)])
        default:
            throw SourceFailure.unsupported
        }
    }
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
