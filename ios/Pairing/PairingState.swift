import Foundation
import Observation

@MainActor
@Observable
final class PairingState {
    enum Connection {
        case unpaired, paired, checking, connected, offline
    }

    private(set) var connection: Connection
    private(set) var busy = false
    private var pairingError = false
    private let pairRequest: (String) async throws -> Void
    private let checkRequest: () async throws -> Void

    var status: String {
        if pairingError { return "Pairing failed. Check the code and try again." }
        switch connection {
        case .unpaired: return "Not paired"
        case .paired: return "Paired"
        case .checking: return "Connecting…"
        case .connected: return "Paired · Connected"
        case .offline: return "Offline. Check your connection and try again."
        }
    }

    init(paired: Bool, pair: @escaping (String) async throws -> Void, check: @escaping () async throws -> Void) {
        connection = paired ? .paired : .unpaired
        pairRequest = pair
        checkRequest = check
    }

    func pair(_ code: String) async {
        guard !busy else { return }
        busy = true
        pairingError = false
        defer { busy = false }
        do {
            try await pairRequest(code)
            connection = .paired
        } catch { pairingError = true }
    }

    func check() async {
        guard !busy, connection != .unpaired else { return }
        busy = true
        pairingError = false
        connection = .checking
        defer { busy = false }
        do {
            try await checkRequest()
            connection = .connected
        } catch { connection = .offline }
    }
}
