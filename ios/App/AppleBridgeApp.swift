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
    @State private var pairing = PairingState()
    var body: some View {
        NavigationStack {
            List {
                Section("Gateway") {
                    NavigationLink("Pairing") { PairingScreen(state: pairing) }
                    Text(pairing.status)
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
            .task {
                if let index = ProcessInfo.processInfo.arguments.firstIndex(of: "--pair-code"), ProcessInfo.processInfo.arguments.count > index + 1 {
                    await pairing.pair(ProcessInfo.processInfo.arguments[index + 1])
                }
            }
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
