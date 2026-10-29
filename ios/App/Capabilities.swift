import Foundation

@MainActor
enum Capabilities {
    static let all: [Capability] = [
        AlarmsCapability.descriptor,
        NotificationsCapability.descriptor,
        ApprovalsCapability.descriptor,
        LocationCapability.descriptor,
        HealthReadCapability.descriptor,
        HealthWriteCapability.descriptor,
        MotionCapability.descriptor,
        DeviceContextCapability.descriptor,
        ScreenTimeCapability.descriptor,
        SiriCapability.descriptor,
        WidgetsCapability.descriptor,
        CaptureCapability.descriptor,
        ContextCapability.descriptor,
        ExtrasCapability.descriptors[0],
        ExtrasCapability.descriptors[1],
        ExtrasCapability.descriptors[2],
        HomeCapability.descriptor,
        AppleApprovalCapability.descriptor,
    ]
}
