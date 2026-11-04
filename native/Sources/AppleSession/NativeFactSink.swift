import NativeProtocol
import Foundation
import SourceSupport

actor NativeFactSink: ObservationSink {
    private var records: [String: JSON] = [:]
    private var scope: [String] = []
    private var generation = ""
    private var checkpoint = ""
    private var complete = false
    private var handles: [String: String] = [:]

    func sourceState(source: String, state: String, coverage: JSON, capabilities: [String], reason: String) async throws {}

    func emit(source: String, generation: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {
        self.generation = generation
        self.scope = scope
        complete = completionEvidence == "complete"
        handles.merge(blobHandles) { _, new in new }
        for row in observations where ["upsert", "absent", "evict"].contains(row["kind"].string) {
            records[row["collection"].string + "/" + row["native_key"].string] = row
        }
    }

    func emitCheckpoint(source: String, generation: String, checkpoint: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {
        self.checkpoint = checkpoint
        try await emit(source: source, generation: generation, observations: observations, scanID: scanID, scanStartedAt: scanStartedAt, scope: scope, completionEvidence: completionEvidence, blobHandles: blobHandles, live: live)
    }

    func result(collection: String? = nil) -> NativeFacts {
        var items = records.keys.sorted().compactMap { records[$0] }.filter { collection == nil || $0["collection"].string == collection }
        if collection == nil || collection == "contacts.contact" || collection == "calendar.event" {
            items += handles.keys.sorted().map { JSON.object(["kind": .string("blob_handle"), "digest": .string($0), "handle": .string(handles[$0] ?? "")]) }
        }
        return NativeFacts(items: items, scope: .object(["collections": .array((collection.map { [$0] } ?? scope).map(JSON.string)), "history_token": .string(checkpoint)]), generation: generation, complete: complete)
    }
}
