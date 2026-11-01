import SwiftUI

@main
struct AppleBridgeApp: App {
    @UIApplicationDelegateAdaptor(BridgeAppDelegate.self) private var delegate
    var body: some Scene {
        WindowGroup {
            #if DEBUG
            Group {
                if RealQualification.active { RealQualificationScreen() }
                else if ShellDeliveryQualification.active { ShellDeliveryQualificationScreen() }
                else { BridgeSettings() }
            }
            .onAppear { UIApplication.shared.isIdleTimerDisabled = true }
            #else
            BridgeSettings()
            #endif
        }
    }
}

struct BridgeSettings: View {
    @Environment(\.scenePhase) private var scenePhase
    @State private var pairing = PairingState()
    @Bindable private var approval = ApprovalReview.shared
    var body: some View {
        TabView {
            ForEach(ShellModel.tabs) { tab in
                Group {
                    switch tab {
                    case .settings: settings
                    case .approvals: ShellApprovalsScreen()
                    case .diagnostics:
                        NavigationStack { DiagnosticsScreen(pairing: pairing) }
                    }
                }
                .tabItem { Label(tab.title, systemImage: tab.symbol) }
            }
        }
        .sheet(isPresented: Binding(get: { approval.job != nil }, set: { if !$0 { approval.job = nil } })) {
            if let job = approval.job { ApprovalReviewScreen(job: job) }
        }
        .onChange(of: scenePhase) { _, phase in
            if phase == .active && !RealQualification.active {
                LocationCapability.resume()
                WidgetsCapability.resume()
                Task { await pairing.check(); await PushDelivery.shared.drain(); await HealthReadController.shared.resume() }
            }
        }
        .task {
            if await RealQualification.runIfRequested() { return }
            LocationCapability.resume()
            WidgetsCapability.resume()
            if let index = ProcessInfo.processInfo.arguments.firstIndex(of: "--pair-code"), ProcessInfo.processInfo.arguments.count > index + 1 {
                await pairing.pair(ProcessInfo.processInfo.arguments[index + 1])
            }
            await pairing.check()
            await PushDelivery.shared.drain()
            await HealthReadController.shared.resume()
        }
    }

    private var settings: some View {
        NavigationStack {
            List {
                Section("Gateway") {
                    NavigationLink("Pairing") { PairingScreen(state: pairing) }
                    Text(pairing.status)
                }
                Section("Capabilities") {
                    ForEach(Capabilities.all) { capability in
                        NavigationLink {
                            if capability.id == "screentime" {
                                ScreenTimeScreen()
                            } else {
                                CapabilityScreen(capability: capability)
                            }
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
                        DiagnosticsScreen(pairing: pairing)
                    }
                }
            }
            .navigationTitle("Apple Bridge")
        }
    }
}

struct DiagnosticsScreen: View {
    @Bindable var pairing: PairingState
    var body: some View {
        Form {
            LabeledContent("Version", value: Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "Unknown")
            LabeledContent("Gateway", value: pairing.status)
            LabeledContent("Capabilities", value: String(Capabilities.all.count))
            LabeledContent("Last command", value: PushDelivery.shared.lastResult)
            LabeledContent("Push delivery", value: PushDelivery.shared.status)
            LabeledContent("Push wakes", value: String(PushDelivery.shared.wakeCount))
            Button("Check pending commands") { Task { await PushDelivery.shared.drain() } }
        }
        .navigationTitle("Diagnostics")
    }
}

#Preview {
    BridgeSettings()
}

struct ShellApprovalsScreen: View {
    @Bindable private var approval = ApprovalReview.shared

    var body: some View {
        NavigationStack {
            TimelineView(.periodic(from: .now, by: 1)) { context in
                let rows = ShellModel.approvals(ApprovalsCapability.pending(), now: Int64(context.date.timeIntervalSince1970))
                List {
                    if rows.isEmpty { Text(ShellModel.emptyApprovals) }
                    ForEach(rows) { row in
                        Button {
                            guard let job = ApprovalsCapability.stored(row.id), job.expires > Int64(Date().timeIntervalSince1970) else { return }
                            approval.job = job
                        } label: {
                            VStack(alignment: .leading) {
                                Text(row.title)
                                Text(row.summary).font(.caption).privacySensitive()
                            }
                        }
                        .disabled(!ApprovalsCapability.enabled)
                    }
                }
            }
            .navigationTitle("Approvals")
            .refreshable { await PushDelivery.shared.drain() }
        }
    }
}
