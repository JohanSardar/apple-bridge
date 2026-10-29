import SwiftUI
import VisionKit

@MainActor
@Observable
final class PairingState {
    var status = "Not paired"
    var busy = false

    init() {
        if let identity = try? DeviceIdentity.load(), identity.deviceID != nil { status = "Paired" }
    }

    func pair(_ code: String) async {
        busy = true
        defer { busy = false }
        do {
            _ = try await PairingClient.pair(code)
            status = "Paired"
        } catch { status = "Pairing failed: \(error.localizedDescription)" }
    }

    func check() async {
        do {
            _ = try await PairingClient.signedRequest(identity: DeviceIdentity.load(), path: "/v1/iphone/status")
            status = "Paired · Connected"
        } catch { status = "Disconnected or revoked" }
    }
}

struct PairingScreen: View {
    @Bindable var state: PairingState
    @State private var code = ""
    @State private var scanning = false

    var body: some View {
        Form {
            LabeledContent("Status", value: state.status)
            Section("Pair with your Mac") {
                TextField("Pairing code", text: $code, axis: .vertical)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .privacySensitive()
                Button("Pair") { Task { await state.pair(code); code = "" } }
                    .disabled(code.isEmpty || state.busy)
                if DataScannerViewController.isSupported && DataScannerViewController.isAvailable {
                    Button("Scan gateway QR") { scanning = true }
                }
            }
            Button("Check connection") { Task { await state.check() } }
            Text("Revoke this iPhone from the Mac with the gateway revoke command.")
        }
        .navigationTitle("Pairing")
        .sheet(isPresented: $scanning) {
            QRScanner { value in
                scanning = false
                Task { await state.pair(value) }
            }
        }
    }
}

struct QRScanner: UIViewControllerRepresentable {
    let scanned: (String) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(scanned: scanned) }
    func makeUIViewController(context: Context) -> DataScannerViewController {
        let scanner = DataScannerViewController(recognizedDataTypes: [.barcode(symbologies: [.qr])], recognizesMultipleItems: false)
        scanner.delegate = context.coordinator
        try? scanner.startScanning()
        return scanner
    }
    func updateUIViewController(_ uiViewController: DataScannerViewController, context: Context) {}
    static func dismantleUIViewController(_ uiViewController: DataScannerViewController, coordinator: Coordinator) { uiViewController.stopScanning() }

    final class Coordinator: NSObject, DataScannerViewControllerDelegate {
        let scanned: (String) -> Void
        var completed = false
        init(scanned: @escaping (String) -> Void) { self.scanned = scanned }
        func dataScanner(_ dataScanner: DataScannerViewController, didAdd addedItems: [RecognizedItem], allItems: [RecognizedItem]) {
            guard !completed else { return }
            for item in addedItems {
                if case .barcode(let barcode) = item, let value = barcode.payloadStringValue, value.hasPrefix("ab1-") {
                    completed = true
                    scanned(value)
                    return
                }
            }
        }
    }
}
