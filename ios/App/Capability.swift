import SwiftUI

struct Capability: Identifiable {
    let id: String
    let title: String
    var permission: String = "Not requested"
    var available: Bool = false
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
                LabeledContent("Permission", value: capability.permission)
                LabeledContent("Last activity", value: "None")
            } footer: {
                if !capability.available {
                    Text("This capability is awaiting its implementation.")
                }
            }
        }
        .navigationTitle(capability.title)
    }
}
