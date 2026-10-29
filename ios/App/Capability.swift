import SwiftUI

typealias IPhoneCommandHandler = @MainActor (Data) async throws -> Data
typealias IPhoneDeferredCommandHandler = @MainActor (PhoneJob) async throws -> Data?

struct Capability: Identifiable {
    let id: String
    let title: String
    var permission: String = "Not requested"
    var available: Bool = false
    var handlers: [String: IPhoneCommandHandler] = [:]
    var deferredHandlers: [String: IPhoneDeferredCommandHandler] = [:]
}

struct CapabilityScreen: View {
    let capability: Capability
    @AppStorage private var enabled: Bool

    init(capability: Capability) {
        self.capability = capability
        _enabled = AppStorage(wrappedValue: false, "capability.\(capability.id).enabled")
    }

    var body: some View {
        Form {
            Section {
                Toggle("Enabled", isOn: $enabled)
                    .disabled(!capability.available)
                    .onChange(of: enabled) { _, value in
                        if value && capability.id == "notifications" {
                            Task { try? await NotificationsCapability.requestPermission() }
                        }
                    }
                if capability.id == "location" {
                    Button("Allow Location") { LocationCapability.adapter.requestPermission() }
                    Text("Enable Always and Precise Location for geofences and significant changes.")
                }
                if capability.id == "healthwrite" {
                    LabeledContent("Health", value: HealthWriteController.shared.status)
                    Button("Allow Health write access") { Task { await HealthWriteController.shared.authorize() } }
                    Button("Test tagged Health write and cleanup") { Task { await HealthWriteController.shared.prove() } }
                }
                if capability.id == "healthread" {
                    LabeledContent("Health", value: HealthReadController.shared.status)
                    Button("Allow Health read access") { Task { await HealthReadController.shared.enable() } }
                    Button("Capture Health updates") { Task { await HealthReadController.shared.resume() } }
                }
                LabeledContent("Permission", value: capability.permission)
                LabeledContent("Last activity", value: "None")
                if capability.id == "journal" { JournalSuggestionPicker() }
                if let feature = ApprovalFeature(rawValue: capability.id) {
                    ApprovalAuthorizationView(feature: feature)
                }
            } footer: {
                if !capability.available {
                    Text(capability.permission)
                }
            }
        }
        .navigationTitle(capability.title)
        .onChange(of: enabled) { _, value in
            if capability.id == "healthread" {
                Task {
                    if value { await HealthReadController.shared.enable() }
                    else { await HealthReadController.shared.disable() }
                }
            }
            if capability.id == "location" {
                if value { LocationCapability.resume() }
                else {
                    if let store = try? LocationCapability.load() {
                        for fence in store.snapshot.fences { LocationCapability.adapter.remove(fence.name) }
                    }
                    try? LocationCapability.adapter.significant(false)
                }
            }
            if capability.id == "locationPush" {
                LocationPushState.setEnabled(value && AppleApprovalCapability.approved(.locationPush))
            }
        }
        .task {
            if capability.id == "locationPush" {
                LocationPushState.setEnabled(enabled && AppleApprovalCapability.approved(.locationPush))
            }
        }
    }
}
