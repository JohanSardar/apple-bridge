import SwiftUI

@main
struct AppleBridgeApp: App {
    var body: some Scene {
        WindowGroup {
            BridgeSettings()
        }
    }
}

struct BridgeSettings: View {
    var body: some View {
        NavigationStack {
            List {
                Section("Gateway") {
                    LabeledContent("Pairing", value: "Not paired")
                }
                Section("Capabilities") {
                    ForEach(Capabilities.all) { capability in
                        NavigationLink {
                            CapabilityScreen(capability: capability)
                        } label: {
                            VStack(alignment: .leading) {
                                Text(capability.title)
                                Text("\(capability.permission) · No activity")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                }
                Section {
                    NavigationLink("Diagnostics") {
                        DiagnosticsScreen()
                    }
                }
            }
            .navigationTitle("Apple Bridge")
        }
    }
}

struct DiagnosticsScreen: View {
    var body: some View {
        Form {
            LabeledContent("Version", value: Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "Unknown")
            LabeledContent("Gateway", value: "Not connected")
            LabeledContent("Capabilities", value: String(Capabilities.all.count))
            LabeledContent("Last command", value: "None")
        }
        .navigationTitle("Diagnostics")
    }
}

#Preview {
    BridgeSettings()
}
