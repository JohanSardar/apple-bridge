import DeviceActivity

final class BridgeMonitor: DeviceActivityMonitor {
    override func intervalDidStart(for activity: DeviceActivityName) {
        guard activity.rawValue == "bridge.downtime" else { return }
        ScreenTimeState.apply(ScreenTimeState.downtime)
    }

    override func intervalDidEnd(for activity: DeviceActivityName) {
        guard activity.rawValue == "bridge.downtime" else { return }
        ScreenTimeState.downtime.clearAllSettings()
    }
}
