# Mac capture

The mac source captures audio devices, display appearance and geometry, power, network path and running applications. Grants are mac:read and mac:write; this slice exposes no mutations. Capture uses bounded notification streams, CoreAudio listeners, workspace notifications, IOPS and NWPathMonitor. Unknown device volume and mute are omitted. Night Shift is omitted. SSID is included only with existing Location authorization; capture never requests it. Time remaining is seconds when IOPS supplies an estimate. Keep-awake describes system idle-sleep assertions.

Limitations: VPN is an interface heuristic, not an authenticated tunnel health check. Real battery parity and five-minute CPU measurement remain unverified.
