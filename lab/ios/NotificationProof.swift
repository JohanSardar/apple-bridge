import Foundation

@MainActor
final class NotificationProofState {
    var operations = 0
    var completions = 0
}

@main
struct NotificationProof {
    @MainActor
    static func main() async {
        let state = NotificationProofState()
        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            DispatchQueue.global().async {
                precondition(!Thread.isMainThread)
                NotificationCompletion.deliver({
                    MainActor.preconditionIsolated()
                    precondition(Thread.isMainThread)
                    await Task.yield()
                    state.operations += 1
                }, completion: {
                    MainActor.assumeIsolated {
                        precondition(Thread.isMainThread)
                        precondition(state.operations == 1)
                        state.completions += 1
                    }
                    continuation.resume()
                })
            }
        }
        precondition(state.completions == 1)
        print("PASS")
    }
}
