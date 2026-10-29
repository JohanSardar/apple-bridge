import DeviceActivity
import SwiftUI

@main
struct BridgeReportExtension: DeviceActivityReportExtension {
    var body: some DeviceActivityReportScene {
        BridgeUsageReport { rows in UsageView(rows: rows) }
    }
}

struct UsageRow: Identifiable {
    let id = UUID()
    let name: String
    let duration: TimeInterval
}

nonisolated struct BridgeUsageReport: DeviceActivityReportScene {
    let context = DeviceActivityReport.Context("bridge.usage")
    let content: ([UsageRow]) -> UsageView

    func makeConfiguration(representing data: DeviceActivityResults<DeviceActivityData>) async -> [UsageRow] {
        var rows: [UsageRow] = []
        for await device in data {
            for await segment in device.activitySegments {
                for await category in segment.categories {
                    for await app in category.applications {
                        rows.append(UsageRow(name: app.application.localizedDisplayName ?? "App", duration: app.totalActivityDuration))
                    }
                    for await site in category.webDomains {
                        rows.append(UsageRow(name: site.webDomain.domain ?? "Website", duration: site.totalActivityDuration))
                    }
                }
            }
        }
        return rows
    }
}

struct UsageView: View {
    let rows: [UsageRow]
    var body: some View {
        List(rows) { row in
            LabeledContent(row.name, value: Duration.seconds(row.duration).formatted(.time(pattern: .hourMinuteSecond)))
        }
        .overlay { if rows.isEmpty { Text("No usage available for this day") } }
    }
}
