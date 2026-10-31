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
    private var generation = UUID().uuidString
    private var path: NWPath?
    private var committed: [String: JSON] = [:]
    public init() {}
    public func open() async -> SourceReadiness {
        var missing: [JSON] = [.string("Night Shift has no supported public API"), .string("SSID requires Location authorization")]
        if NSPasteboard.general.accessBehavior != .alwaysAllow { missing.append(.string("Clipboard requires Always Allow in Paste from Other Apps")) }
        return SourceReadiness(state: "ready", coverage: .object(["fields": .string("partial"), "missing": .array(missing)]), capabilities: ["mac.set_volume", "mac.set_mute", "mac.set_output_device", "mac.set_appearance", "mac.launch_app", "mac.activate_app", "mac.quit_app", "mac.keep_awake", "mac.clipboard_write", "mac.clipboard_read", "mac.notify", "mac.open_settings", "mac.set_wallpaper", "mac.network_check", "mac.bluetooth_devices", "mac.time_machine_status", "mac.recognize_song"])
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
        try await scan(sink, live: false)
        for await _ in changes {
            try Task.checkCancellation()
            let current = Set(try audio().compactMap { UInt32($0.0) } + [AudioObjectID(kAudioObjectSystemObject)])
            for device in devices.subtracting(current) { for var address in addresses(device) { AudioObjectRemovePropertyListenerBlock(device, &address, .main, listener) } }
            for device in current.subtracting(devices) { for var address in addresses(device) { AudioObjectAddPropertyListenerBlock(device, &address, .main, listener) } }
            devices = current
            try await scan(sink, live: true)
        }
    }
    private func script(_ text: String) throws -> String {
        var error: NSDictionary?
        guard let script = NSAppleScript(source: text) else { throw SourceFailure.invalid }
        let result = script.executeAndReturnError(&error)
        guard error == nil else { throw SourceFailure.unavailable }
        return result.stringValue ?? ""
    }
    public func prepare(_ command: JSON) async throws -> Prepared {
        let action = command["action"]
        guard (await open()).capabilities.contains(action["op"].string) else { throw SourceFailure.unsupported }
        if action["op"].string == "mac.clipboard_write", NSPasteboard.general.accessBehavior != .alwaysAllow { throw SourceFailure.permissionMissing }
        return Prepared(nativeState: .null, fingerprint: command["request_hash"].string, action: action)
    }
    public func dispatch(_ prepared: Prepared) async throws -> JSON {
        let args = prepared.action["args"]
        var readback: JSON = .null
        switch prepared.action["op"].string {
        case "mac.lock_screen", "mac.sleep_display", "mac.restart", "mac.shutdown":
            throw SourceFailure.unsupported
        case "mac.iwork_export":
 throw SourceFailure.unsupported
 case "mac.notify":
            let center = UNUserNotificationCenter.current()
            let settings = await center.notificationSettings()
            guard settings.authorizationStatus == .authorized || settings.authorizationStatus == .provisional else { throw SourceFailure.permissionMissing }
            let content = UNMutableNotificationContent()
            content.title = args["title"].string
            content.body = args["body"].string
            guard !content.title.isEmpty, content.title.utf8.count <= 256, content.body.utf8.count <= 4096 else { throw SourceFailure.invalid }
            if !args["action_url"].string.isEmpty {
                guard let url = URL(string: args["action_url"].string), ["https", "http"].contains(url.scheme ?? ""), url.host != nil else { throw SourceFailure.invalid }
                content.userInfo = ["action_url": url.absoluteString]
            }
            let identifier = "apple-bridge-" + UUID().uuidString
            try await center.add(UNNotificationRequest(identifier: identifier, content: content, trigger: nil))
            var delivered = false
            for _ in 0..<20 {
                delivered = await center.deliveredNotifications().contains { $0.request.identifier == identifier }
                if delivered { break }
                try await Task.sleep(for: .milliseconds(100))
            }
            guard delivered else { throw SourceFailure.unavailable }
            readback = .object(["delivered": .bool(true), "identifier": .string(identifier)])
        case "mac.open_settings":
            let panes = ["general": "com.apple.systempreferences.GeneralSettings", "notifications": "com.apple.Notifications-Settings.extension", "network": "com.apple.Network-Settings.extension", "displays": "com.apple.Displays-Settings.extension", "wallpaper": "com.apple.Wallpaper-Settings.extension", "privacy": "com.apple.settings.PrivacySecurity.extension"]
            guard let pane = panes[args["pane"].string], let url = URL(string: "x-apple.systempreferences:" + pane), NSWorkspace.shared.open(url) else { throw SourceFailure.invalid }
            var active = false
            for _ in 0..<30 {
                active = NSWorkspace.shared.frontmostApplication?.bundleIdentifier == "com.apple.systempreferences"
                if active { break }
                try await Task.sleep(for: .milliseconds(100))
            }
            guard active else { throw SourceFailure.unavailable }
            readback = .object(["frontmost": .bool(true), "pane": args["pane"]])
        case "mac.set_wallpaper":
            guard let bytes = Data(base64Encoded: args["image_blob"].string), !bytes.isEmpty, bytes.count <= 8 * 1024 * 1024, let bitmap = NSBitmapImageRep(data: bytes), let png = bitmap.representation(using: .png, properties: [:]), !NSScreen.screens.isEmpty else { throw SourceFailure.invalid }
            let directory = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("AppleBridge/wallpapers")
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            let url = directory.appendingPathComponent(UUID().uuidString + ".png")
            try png.write(to: url, options: .atomic)
            for screen in NSScreen.screens { try NSWorkspace.shared.setDesktopImageURL(url, for: screen, options: [:]) }
            guard NSScreen.screens.allSatisfy({ NSWorkspace.shared.desktopImageURL(for: $0) == url }) else { throw SourceFailure.conflict }
            readback = .object(["matches": .bool(true)])
        case "mac.clipboard_write":
            let board = NSPasteboard.general
            guard board.accessBehavior == .alwaysAllow else { throw SourceFailure.permissionMissing }
            let text = args["text"].string
            guard text.utf8.count <= 65536 else { throw SourceFailure.invalid }
            let item = NSPasteboardItem()
            guard item.setString(text, forType: .string), item.setData(Data(), forType: NSPasteboard.PasteboardType("org.nspasteboard.TransientType")), item.setData(Data(), forType: NSPasteboard.PasteboardType("org.nspasteboard.ConcealedType")) else { throw SourceFailure.unavailable }
            board.clearContents()
            guard board.writeObjects([item]), board.string(forType: .string) == text else { throw SourceFailure.conflict }
            readback = .bool(true)
        case "mac.set_volume":
            let volume = args["volume"].number
            guard volume >= 0, volume <= 100 else { throw SourceFailure.invalid }
            _ = try script("set volume output volume " + String(Int(volume)))
            let result = try script("output volume of (get volume settings)")
            guard let actual = Double(result), abs(actual - volume) <= 1 else { throw SourceFailure.conflict }
            readback = .number(actual)
        case "mac.set_mute":
            let muted = args["muted"].bool
            _ = try script("set volume output muted " + (muted ? "true" : "false"))
            let result = try script("output muted of (get volume settings)")
            guard (result == "true") == muted else { throw SourceFailure.conflict }
            readback = .bool(muted)
        case "mac.set_output_device":
            guard var device = UInt32(args["device"].string), try audio().contains(where: { $0.0 == String(device) && $0.1["output"].bool }) else { throw SourceFailure.invalid }
            var address = AudioObjectPropertyAddress(mSelector: kAudioHardwarePropertyDefaultOutputDevice, mScope: kAudioObjectPropertyScopeGlobal, mElement: 0)
            guard AudioObjectSetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, 4, &device) == noErr else { throw SourceFailure.unavailable }
            var actual: UInt32 = 0; var count: UInt32 = 4
            guard AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &count, &actual) == noErr, actual == device else { throw SourceFailure.conflict }
            readback = .string(String(actual))
        case "mac.set_appearance":
            if NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.systemevents").isEmpty {
                guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "com.apple.systemevents") else { throw SourceFailure.unavailable }
                let configuration = NSWorkspace.OpenConfiguration()
                configuration.activates = false
                _ = try await NSWorkspace.shared.openApplication(at: url, configuration: configuration)
            }
            let target = NSAppleEventDescriptor(bundleIdentifier: "com.apple.systemevents")
            guard AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), false) == noErr else { throw SourceFailure.permissionMissing }
            let value = args["appearance"].string
            guard value == "dark" || value == "light" else { throw SourceFailure.invalid }
            _ = try script("tell application \"System Events\" to tell appearance preferences to set dark mode to " + (value == "dark" ? "true" : "false"))
            let actual = try script("tell application \"System Events\" to tell appearance preferences to get dark mode")
            guard (actual == "true") == (value == "dark") else { throw SourceFailure.conflict }
            readback = .string(value)
        case "mac.launch_app", "mac.activate_app", "mac.quit_app":
            let bundle = args["bundle_id"].string
            guard !bundle.isEmpty else { throw SourceFailure.invalid }
            let op = prepared.action["op"].string
            if op == "mac.launch_app" {
                guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundle) else { throw SourceFailure.invalid }
                _ = try await NSWorkspace.shared.openApplication(at: url, configuration: NSWorkspace.OpenConfiguration())
            } else {
                guard let app = NSRunningApplication.runningApplications(withBundleIdentifier: bundle).first else { throw SourceFailure.conflict }
                if op == "mac.quit_app" { guard app.terminate() else { throw SourceFailure.unavailable } }
                else { guard app.activate() else { throw SourceFailure.unavailable } }
            }
            for _ in 0..<50 {
                let apps = NSRunningApplication.runningApplications(withBundleIdentifier: bundle)
                let matches = op == "mac.quit_app" ? apps.isEmpty : op == "mac.activate_app" ? apps.contains(where: { $0.isActive }) : !apps.isEmpty
                if matches { readback = .object(["bundle_id": .string(bundle), "running": .bool(!apps.isEmpty)]); break }
                try await Task.sleep(for: .milliseconds(100))
            }
            guard readback != .null else { throw SourceFailure.conflict }
        case "mac.keep_awake":
            expiry?.cancel(); expiry = nil
            if assertion != 0 { IOPMAssertionRelease(assertion); assertion = 0 }
            if args["on"].bool {
                guard IOPMAssertionCreateWithName(kIOPMAssertionTypePreventUserIdleSystemSleep as CFString, IOPMAssertionLevel(kIOPMAssertionLevelOn), "Apple Bridge keep awake" as CFString, &assertion) == kIOReturnSuccess else { throw SourceFailure.unavailable }
                let duration = args["duration"].number
                if duration > 0 {
                    guard duration <= 86400 else { IOPMAssertionRelease(assertion); assertion = 0; throw SourceFailure.invalid }
                    let held = assertion
                    expiry = Task { [weak self] in
                        do { try await Task.sleep(for: .seconds(duration)) } catch { return }
                        guard !Task.isCancelled, let self, self.assertion == held else { return }
                        if self.assertion != 0 { IOPMAssertionRelease(self.assertion); self.assertion = 0 }
                    }
                }
            }
            if assertion != 0 { guard IOPMAssertionCopyProperties(assertion) != nil else { throw SourceFailure.conflict } }
            readback = .bool(assertion != 0)
        default: throw SourceFailure.unsupported
        }
        return .object(["disposition": .string("locally_observed"), "observed": .bool(true), "fingerprint": .string(prepared.fingerprint), "readback": readback])
    }
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
