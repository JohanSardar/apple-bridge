import Foundation
import FamilyControls
import DeviceActivity
import SwiftUI

@MainActor
enum ScreenTimeCapability {
    static let descriptor = Capability(
        id: "screentime", title: "Screen Time", permission: "Allow required", available: true,
        handlers: [
            "iphone.screentime.shield": { _ in
                try authorized()
                ScreenTimeState.apply(ScreenTimeState.manual)
                return Data("{\"shielded\":true}".utf8)
            },
            "iphone.screentime.unshield": { _ in
                try authorized()
                ScreenTimeState.manual.clearAllSettings()
                return Data("{\"shielded\":false}".utf8)
            },
            "iphone.screentime.schedule": { data in
                try authorized()
                let args = try JSONDecoder().decode(DowntimeArguments.self, from: data)
                try schedule(start: args.startMinute, end: args.endMinute)
                return Data("{\"scheduled\":true}".utf8)
            },
            "iphone.screentime.cancel": { _ in
                try authorized()
                DeviceActivityCenter().stopMonitoring([.init("bridge.downtime")])
                ScreenTimeState.downtime.clearAllSettings()
                return Data("{\"scheduled\":false}".utf8)
            }
        ])

    struct DowntimeArguments: Decodable {
        let startMinute: Int
        let endMinute: Int
        enum CodingKeys: String, CodingKey {
            case startMinute = "start_minute"
            case endMinute = "end_minute"
        }
    }

    static func authorized() throws {
        guard BridgeShieldPolicy.authorized(status: AuthorizationCenter.shared.authorizationStatus.rawValue) else { throw ScreenTimeError.authorization }
    }

    static func schedule(start: Int, end: Int) throws {
        guard BridgeShieldPolicy.validSchedule(start: start, end: end) else { throw ScreenTimeError.schedule }
        try DeviceActivityCenter().startMonitoring(.init("bridge.downtime"), during: DeviceActivitySchedule(
            intervalStart: DateComponents(hour: start / 60, minute: start % 60),
            intervalEnd: DateComponents(hour: end / 60, minute: end % 60), repeats: true))
    }
}

enum ScreenTimeError: Error {
    case authorization
    case schedule
}

struct ScreenTimeScreen: View {
    @AppStorage("capability.screentime.enabled") private var enabled = false
    @State private var selection = ScreenTimeState.selection
    @State private var picker = false
    @State private var status = "Allow required"
    @State private var start = Calendar.current.startOfDay(for: Date())
    @State private var end = Calendar.current.startOfDay(for: Date()).addingTimeInterval(3600)

    var body: some View {
        Form {
            Toggle("Enabled", isOn: $enabled)
            Text(status)
            Button("Authorize Screen Time") {
                Task {
                    do {
                        try await AuthorizationCenter.shared.requestAuthorization(for: .individual)
                        status = "Authorized"
                    } catch { status = "Authorization unavailable" }
                }
            }
            Button("Choose apps, categories and websites") { picker = true }
            Button("Shield selection") { perform { try ScreenTimeCapability.authorized(); ScreenTimeState.apply(ScreenTimeState.manual) } }
            Button("Remove Bridge shields") { perform { ScreenTimeState.manual.clearAllSettings() } }
            DatePicker("Downtime start", selection: $start, displayedComponents: .hourAndMinute)
            DatePicker("Downtime end", selection: $end, displayedComponents: .hourAndMinute)
            Button("Schedule Bridge downtime") {
                perform {
                    try ScreenTimeCapability.authorized()
                    let calendar = Calendar.current
                    try ScreenTimeCapability.schedule(start: calendar.component(.hour, from: start) * 60 + calendar.component(.minute, from: start), end: calendar.component(.hour, from: end) * 60 + calendar.component(.minute, from: end))
                }
            }
            Button("Cancel Bridge downtime") {
                DeviceActivityCenter().stopMonitoring([.init("bridge.downtime")])
                ScreenTimeState.downtime.clearAllSettings()
            }
            NavigationLink("App and website usage") {
                DeviceActivityReport(.init("bridge.usage"), filter: DeviceActivityFilter(segment: .daily(during: Calendar.current.dateInterval(of: .day, for: Date())!), users: .all, devices: .init([.iPhone])))
            }
            Text("Only Bridge shields and schedules are changed. Usage stays in this report.")
        }
        .navigationTitle("Screen Time")
        .familyActivityPicker(isPresented: $picker, selection: $selection)
        .onChange(of: selection) { _, value in ScreenTimeState.selection = value }
    }

    private func perform(_ action: () throws -> Void) {
        do { try action(); status = "Updated" } catch { status = "Unable to update; check authorization and schedule" }
    }
}
