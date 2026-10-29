import ActivityKit
import AlarmKit
import SwiftUI
import WidgetKit

@main
struct AlarmActivity: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: AlarmAttributes<BridgeAlarmMetadata>.self) { context in
            HStack {
                Image(systemName: "alarm.fill")
                Text("Apple Bridge")
                Spacer()
                countdown(context.state)
            }
            .padding()
            .activityBackgroundTint(.orange.opacity(0.2))
        } dynamicIsland: { context in
            DynamicIsland {
                DynamicIslandExpandedRegion(.leading) { Image(systemName: "alarm.fill") }
                DynamicIslandExpandedRegion(.trailing) { countdown(context.state) }
                DynamicIslandExpandedRegion(.bottom) { Text("Apple Bridge") }
            } compactLeading: {
                Image(systemName: "alarm.fill")
            } compactTrailing: {
                countdown(context.state)
            } minimal: {
                Image(systemName: "alarm.fill")
            }
        }
    }

    @ViewBuilder
    private func countdown(_ state: AlarmPresentationState) -> some View {
        switch state.mode {
        case .countdown(let countdown):
            Text(timerInterval: countdown.startDate...countdown.fireDate, countsDown: true)
                .monospacedDigit()
        case .paused:
            Text("Paused")
        case .alert:
            Text("Alarm")
        @unknown default:
            Text("Alarm")
        }
    }
}
