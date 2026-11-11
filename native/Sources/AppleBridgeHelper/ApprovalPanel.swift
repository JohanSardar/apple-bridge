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
        PanelCard(calm: true) {
            VStack(spacing: 0) {
                HStack(spacing: 14) {
                    tile
                    Image(systemName: "arrow.left.arrow.right").font(.system(size: 15, weight: .semibold)).foregroundStyle(.secondary)
                    Image(nsImage: GuideController.appIcon).resizable().frame(width: 58, height: 58)
                }
                .padding(.top, 26)
                if let request = model.request {
                    Text("\(request.client_name) wants to use Apple Bridge").font(.system(size: 19, weight: .bold)).multilineTextAlignment(.center).fixedSize(horizontal: false, vertical: true).padding(.top, 16)
                    Text(request.from).font(.system(size: 12, weight: .medium)).foregroundStyle(.secondary).padding(.top, 4)
                    if let access = request.access, !access.isEmpty {
                        Text(access).font(.system(size: 12)).multilineTextAlignment(.center).fixedSize(horizontal: false, vertical: true)
                            .padding(10).frame(maxWidth: .infinity)
                            .background(RoundedRectangle(cornerRadius: 10, style: .continuous).fill(Color.primary.opacity(0.06)))
                            .padding(.top, 12)
                    }
                    if request.show_code == true {
                        VStack(spacing: 2) {
                            Text("Your browser should show").font(.system(size: 11, weight: .medium)).foregroundStyle(.secondary)
                            Text(request.code).font(.system(size: 26, weight: .bold, design: .rounded)).tracking(4)
                        }
                        .padding(.top, 10)
                    }
                    if !model.warning.isEmpty {
                        Label(model.warning, systemImage: "exclamationmark.triangle.fill").font(.system(size: 11)).foregroundStyle(.orange).padding(.top, 8)
                    }
                    Spacer(minLength: 12)
                    HStack(spacing: 10) {
                        Button("Deny") { model.decide(request, false) }.buttonStyle(PillButtonStyle())
                        Button("Allow") { model.decide(request, true) }.buttonStyle(PillButtonStyle(prominent: true))
                    }
                    .padding(.bottom, 24)
                } else {
                    if model.outcome.hasSuffix("is connected") { AllowedMark().padding(.top, 14) }
                    Text(model.outcome.isEmpty ? "Connected" : model.outcome).font(.system(size: 19, weight: .bold)).multilineTextAlignment(.center).padding(.top, 10)
                    Spacer(minLength: 12)
                }
            }
            .padding(.horizontal, 22)
            .frame(width: 340, height: model.request?.show_code == true ? 360 : 312)
        }
    }

    var tile: some View {
        let name = model.request?.client_name ?? model.decided
        let initial = name.first(where: { $0.isLetter || $0.isNumber }).map { String($0).uppercased() } ?? "?"
        return RoundedRectangle(cornerRadius: 14, style: .continuous)
            .fill(LinearGradient(colors: [Palette.sky, Palette.lagoon], startPoint: .topLeading, endPoint: .bottomTrailing))
            .frame(width: 58, height: 58)
            .overlay(Text(initial).font(.system(size: 26, weight: .bold)).foregroundStyle(.white))
    }
}
