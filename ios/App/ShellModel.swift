import Foundation

struct ShellApprovalRow: Identifiable, Equatable {
    let id: String
    let title: String
    let summary: String
    let expires: Int64
}

enum ShellTab: String, CaseIterable, Identifiable {
    case settings, approvals, diagnostics

    var id: String { rawValue }
    var title: String {
        switch self {
        case .settings: return "Settings"
        case .approvals: return "Approvals"
        case .diagnostics: return "Diagnostics"
        }
    }
    var symbol: String {
        switch self {
        case .settings: return "gearshape"
        case .approvals: return "checkmark.shield"
        case .diagnostics: return "waveform.path.ecg"
        }
    }
}

enum ShellModel {
    static let tabs = ShellTab.allCases
    static let emptyApprovals = "No pending approvals"

    static func approvals(_ jobs: [PhoneJob], now: Int64) -> [ShellApprovalRow] {
        jobs.filter { $0.expires > now }.sorted {
            if $0.expires != $1.expires { return $0.expires < $1.expires }
            return $0.command.command_id < $1.command.command_id
        }.map { job in
            let summary: String
            if case .string(let body) = job.command.args["body"], !body.isEmpty { summary = body }
            else { summary = job.command.op }
            return ShellApprovalRow(id: job.command.command_id,
                                    title: job.command.command_id.hasPrefix("connector:") ? "Review connection" : "Review command",
                                    summary: summary, expires: job.expires)
        }
    }
}
