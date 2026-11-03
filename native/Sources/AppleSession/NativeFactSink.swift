import NativeProtocol
import Foundation
import SourceSupport

actor NativeFactSink: ObservationSink {
    private var records: [String: JSON] = [:]
    private var scope: [String] = []
    private var generation = ""
    private var checkpoint = ""
    private var complete = false

    func sourceState(source: String, state: String, coverage: JSON, capabilities: [String], reason: String) async throws {}

    func emit(source: String, generation: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {
        self.generation = generation
        self.scope = scope
        complete = completionEvidence == "complete"
        for row in observations where row["kind"].string == "upsert" {
            records[row["collection"].string + "/" + row["native_key"].string] = row
        }
    }

    func emitCheckpoint(source: String, generation: String, checkpoint: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {
        self.checkpoint = checkpoint
        try await emit(source: source, generation: generation, observations: observations, scanID: scanID, scanStartedAt: scanStartedAt, scope: scope, completionEvidence: completionEvidence, blobHandles: blobHandles, live: live)
    }

    func result(collection: String? = nil) -> NativeFacts {
        let items = records.keys.sorted().compactMap { records[$0] }.filter { collection == nil || $0["collection"].string == collection }
        return NativeFacts(items: items, scope: .object(["collections": .array((collection.map { [$0] } ?? scope).map(JSON.string)), "history_token": .string(checkpoint)]), generation: generation, complete: complete)
    }
}
