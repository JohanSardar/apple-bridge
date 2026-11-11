import AppKit
import SwiftUI

struct ConnectorRequest: Decodable, Sendable, Equatable {
    let id: String
    let code: String
    let client_name: String
    let redirect_host: String
    let scopes: [String]
    let access: String?
    let expires_at: String
    let show_code: Bool?
    let source: String?

    var from: String {
        switch source {
        case "this_mac": return "An app on this Mac"
        case "tailnet": return "A device on your Tailscale network"
        default:
            let host = redirect_host.split(separator: ":").first.map(String.init) ?? redirect_host
            return ["localhost", "127.0.0.1", "[::1]"].contains(host) ? "An app on the device that started this" : "Returns to " + host
        }
    }
}

@MainActor
final class ApprovalModel: ObservableObject {
    @Published var request: ConnectorRequest?
    @Published var warning = ""
    @Published var outcome = ""
    @Published var decided = ""
    var decide: (ConnectorRequest, Bool) -> Void = { _, _ in }
}

struct ApprovalView: View {
    @ObservedObject var model: ApprovalModel

    var body: some View {
        VStack(spacing: 0) {
            Image(nsImage: GuideController.appIcon).resizable().interpolation(.high).frame(width: 64, height: 64).clipShape(RoundedRectangle(cornerRadius: 14.5, style: .continuous))
            if let request = model.request {
                Text("Allow \u{201C}\(request.client_name)\u{201D} to use your Mac?")
                    .font(.system(size: 13, weight: .bold)).multilineTextAlignment(.center).fixedSize(horizontal: false, vertical: true)
                    .padding(.top, 14)
                Text(detail(request))
                    .font(.system(size: 11)).multilineTextAlignment(.center).fixedSize(horizontal: false, vertical: true)
                    .padding(.top, 6)
                if request.show_code == true {
                    Text("Allow only if your browser shows").font(.system(size: 11)).foregroundStyle(.secondary).padding(.top, 12)
                    Text(request.code).font(.system(size: 22, weight: .semibold)).monospacedDigit().tracking(3).padding(.top, 2)
                }
                if !model.warning.isEmpty {
                    Text(model.warning).font(.system(size: 11)).foregroundStyle(Color(nsColor: .systemOrange)).multilineTextAlignment(.center).fixedSize(horizontal: false, vertical: true).padding(.top, 8)
                }
                HStack(spacing: 8) {
                    Button("Don\u{2019}t Allow") { model.decide(request, false) }.buttonStyle(AlertButtonStyle())
                    Button("Allow") { model.decide(request, true) }.buttonStyle(AlertButtonStyle(prominent: true))
                }
                .padding(.top, 18)
            } else {
                let connected = model.outcome.hasSuffix("is connected")
                Text(model.outcome.isEmpty ? "Connected" : model.outcome)
                    .font(.system(size: 13, weight: .bold)).multilineTextAlignment(.center).fixedSize(horizontal: false, vertical: true)
                    .padding(.top, 14)
                if connected {
                    Label("Allowed", systemImage: "checkmark.circle.fill").font(.system(size: 11)).foregroundStyle(Color(nsColor: .systemGreen)).padding(.top, 6)
                }
            }
        }
        .padding(EdgeInsets(top: 20, leading: 16, bottom: 16, trailing: 16))
        .frame(width: 260)
        .background(AlertBackground())
    }

    func detail(_ request: ConnectorRequest) -> String {
        let access = request.access.map { $0.hasSuffix(".") ? $0 : $0 + "." } ?? ""
        return [access, request.from + "."].filter { $0.count > 1 }.joined(separator: "\n")
    }
}
