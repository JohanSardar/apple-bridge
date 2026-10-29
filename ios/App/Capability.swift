import SwiftUI

typealias IPhoneCommandHandler = @MainActor (Data) async throws -> Data

struct Capability: Identifiable {
    let id: String
    let title: String
    var permission: String = "Not requested"
    var available: Bool = false
    var handlers: [String: IPhoneCommandHandler] = [:]
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
                if capability.id == "journal" { JournalSuggestionPicker() }
            } footer: {
                if !capability.available {
                    Text(capability.permission)
                }
            }
        }
        .navigationTitle(capability.title)
    }
}
