import ActivityKit
import SwiftUI
import WidgetKit

@main
struct BridgeWidgets: WidgetBundle {
    var body: some Widget {
        AgentTaskWidget()
        BridgeStatusWidget()
    }
}

struct AgentTaskWidget: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: AgentTaskAttributes.self) { context in
            VStack(alignment: .leading) {
                Label(context.attributes.title, systemImage: "terminal")
                Text(context.state.status)
                ProgressView(value: context.state.progress)
            }.padding()
        } dynamicIsland: { context in
            DynamicIsland {
                DynamicIslandExpandedRegion(.leading) { Image(systemName: "terminal") }
                DynamicIslandExpandedRegion(.trailing) { Text(context.state.progress, format: .percent.precision(.fractionLength(0))) }
                DynamicIslandExpandedRegion(.bottom) {
                    VStack { Text(context.attributes.title); Text(context.state.status); ProgressView(value: context.state.progress) }
                }
            } compactLeading: { Image(systemName: "terminal") }
            compactTrailing: { Text(context.state.progress, format: .percent.precision(.fractionLength(0))) }
            minimal: { Image(systemName: "terminal") }
        }
    }
}

struct StatusEntry: TimelineEntry {
    let date: Date
    let snapshot: BridgeStatus
}

struct StatusProvider: TimelineProvider {
    func placeholder(in context: Context) -> StatusEntry {
        StatusEntry(date: Date(), snapshot: BridgeStatus(status: "Registered", updated: Date()))
    }
    func getSnapshot(in context: Context, completion: @escaping (StatusEntry) -> Void) {
        completion(StatusEntry(date: Date(), snapshot: .load()))
    }
    func getTimeline(in context: Context, completion: @escaping (Timeline<StatusEntry>) -> Void) {
        let entry = StatusEntry(date: Date(), snapshot: .load())
        completion(Timeline(entries: [entry], policy: .after(Date().addingTimeInterval(900))))
    }
}

struct BridgeStatusView: View {
    @Environment(\.widgetFamily) private var family
    let entry: StatusEntry
    var body: some View {
        if family == .accessoryInline {
            Label("Bridge: \(entry.snapshot.status)", systemImage: "link")
        } else {
            VStack(alignment: .leading) {
                Label("Apple Bridge", systemImage: "link")
                Text(entry.snapshot.status).font(.caption)
                if entry.snapshot.updated != .distantPast {
                    Text(entry.snapshot.updated, style: .relative).font(.caption2)
                }
            }
        }
    }
}

struct BridgeStatusWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "BridgeStatus", provider: StatusProvider()) { entry in
            BridgeStatusView(entry: entry)
            .containerBackground(.fill.tertiary, for: .widget)
        }
        .configurationDisplayName("Bridge status")
        .description("Last reported bridge delivery status and freshness.")
        .supportedFamilies([.systemSmall, .systemMedium, .accessoryRectangular, .accessoryInline])
    }
}
