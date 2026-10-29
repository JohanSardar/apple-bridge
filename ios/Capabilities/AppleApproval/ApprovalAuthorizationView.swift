import SwiftUI
import FinanceKit
import UserNotifications
import CoreLocation

struct ApprovalAuthorizationView: View {
    let feature: ApprovalFeature
    @State private var status = ""
    @State private var locationManager = CLLocationManager()

    var body: some View {
        if AppleApprovalCapability.approved(feature) {
            Button("Request system authorization") {
                Task {
                    do {
                        switch feature {
                        case .wallet:
                            status = String(describing: try await FinanceStore.shared.requestAuthorization())
                        case .criticalAlerts:
                            let allowed = try await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .criticalAlert])
                            status = allowed ? "Authorization granted" : "Authorization denied"
                        case .locationPush:
                            locationManager.requestAlwaysAuthorization()
                            status = "Check Location Services authorization"
                        case .fallDetection:
                            status = "Needs a supported Watch companion"
                        }
                    } catch { status = "Authorization unavailable" }
                }
            }
            if !status.isEmpty { Text(status) }
        }
    }
}
