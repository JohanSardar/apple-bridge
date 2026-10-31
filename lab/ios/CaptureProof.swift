import Foundation

@MainActor
final class FakeCapture: CaptureAdapter {
    var allowed = false
    var captures = 0
    func approve(kind: String, seconds: Int) async throws -> Bool { allowed }
    func capture(kind: String, seconds: Int) async throws -> CaptureBytes {
        captures += 1
        return CaptureBytes(data: Data("[ab-test]".utf8), contentType: kind == "photo" ? "image/jpeg" : "audio/wav")
    }
}

@main
struct CaptureProof {
    @MainActor
    static func main() async throws {
        var active = false
        let activation = Task { @MainActor in
            try await Task.sleep(for: .milliseconds(10))
            active = true
        }
        let ready = await QualificationWait.until(timeout: 1, interval: .milliseconds(1)) { active }
        try await activation.value
        precondition(ready)
        let expired = await QualificationWait.until(timeout: 0) { false }
        precondition(!expired)
        let cancelled = Task { @MainActor in
            await QualificationWait.until(timeout: 1) { false }
        }
        cancelled.cancel()
        let cancelledResult = await cancelled.value
        precondition(!cancelledResult)
        let adapter = FakeCapture()
        var uploads = 0
        do {
            _ = try await CaptureFlow.run(kind: "photo", seconds: 3, adapter: adapter) { _ in uploads += 1; return Data() }
            fatalError()
        } catch {}
        precondition(adapter.captures == 0 && uploads == 0)
        adapter.allowed = true
        for kind in ["photo", "audio"] {
            let result = try await CaptureFlow.run(kind: kind, seconds: 3, adapter: adapter) { bytes in
                precondition(bytes.data == Data("[ab-test]".utf8))
                uploads += 1
                return Data(bytes.contentType.utf8)
            }
            precondition(!result.isEmpty)
        }
        precondition(adapter.captures == 2 && uploads == 2)
        do {
            _ = try await CaptureFlow.run(kind: "audio", seconds: 3, adapter: adapter) { _ in throw CaptureError.invalid }
            fatalError()
        } catch {}
        print("PASS 4")
    }
}
