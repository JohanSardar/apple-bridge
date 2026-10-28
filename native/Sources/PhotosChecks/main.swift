import Foundation
import Darwin
import PhotosSource
import SourceSupport

actor Sink: ObservationSink {
    var count = 0
    var rows: [JSON] = []
    func emitCheckpoint(source: String, generation: String, checkpoint: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws {
        guard source == "photos", scope == ["photos.asset", "photos.album"], completionEvidence == "complete", blobHandles.isEmpty else { throw SourceFailure.corrupt }
        count += 1
        rows = observations
    }
    func emit(source: String, generation: String, observations: [JSON], scanID: String, scanStartedAt: String, scope: [String], completionEvidence: String, blobHandles: [String: String], live: Bool) async throws { throw SourceFailure.corrupt }
    func sourceState(source: String, state: String, coverage: JSON, capabilities: [String], reason: String) async throws {}
}

@main
struct Checks {
    static func main() async throws {
        if CommandLine.arguments.contains("PHO-READ-REAL") {
            let readiness = await PhotosSource().open()
            print("photos_authorization_reason=" + readiness.reason)
            return
        }
        if CommandLine.arguments.contains("PHO-COMMANDS") {
            try await commandChecks()
            return
        }
        let data: JSON = .object(["local_id": .string("synthetic"), "created": .string("2026-01-01T00:00:00Z"), "kind": .string("image"), "size": .null, "duration": .number(0), "hidden": .bool(false), "location": .null, "cloud_state": .string("unknown"), "favorite": .bool(true), "albums": .array([.string("album")])])
        let fixture: [JSON] = [.object(["kind": .string("upsert"), "collection": .string("photos.asset"), "native_key": .string("synthetic"), "data": data]), .object(["kind": .string("upsert"), "collection": .string("photos.album"), "native_key": .string("album"), "data": .object(["name": .string("Synthetic"), "kind": .string("user"), "count": .number(1)])])]
        let bytes = Data("synthetic original".utf8)
        let source = PhotosSource(fixture: fixture, fixtureBlobs: ["photos.original/synthetic": bytes, "photos.thumbnail/synthetic": bytes])
        var handles: [String] = []
        for operation in ["photos.original", "photos.thumbnail"] {
            let blob = try await source.blob(operation, .object(["asset": .string("synthetic")]))
            handles.append(blob["handle"].string)
            guard blob["size"].number == Double(bytes.count), blob["digest"].string.count == 64 else { throw SourceFailure.corrupt }
            let read = try await source.readBlob(blob["handle"].string, offset: 0, length: 131072)
            guard read["data"].string == bytes.base64EncodedString(), read["digest"] == blob["digest"], read["eof"].bool else { throw SourceFailure.corrupt }
        }
        guard await source.open().state == "ready", try await source.snapshot() == fixture else { throw SourceFailure.corrupt }
        let match = try await source.search(.object(["favorite": .bool(true), "album": .string("album"), "kind": .string("image"), "from": .string("2025-01-01T00:00:00Z")]))
        guard match["items"].array == [data] else { throw SourceFailure.corrupt }
        let miss = try await source.search(.object(["favorite": .bool(false)]))
        guard miss["items"].array.isEmpty else { throw SourceFailure.corrupt }
        do { _ = try await source.search(.object(["text": .string("synthetic")])); throw SourceFailure.corrupt } catch SourceFailure.unsupported {}
        let invalid: [JSON] = [.object(["limit": .number(-1)]), .object(["from": .string("bad-date")]), .object(["favorite": .string("true")]), .object(["kind": .bool(true)])]
        for args in invalid {
            do { _ = try await source.search(args); throw SourceFailure.corrupt } catch SourceFailure.invalid {}
        }
        let future = try await source.search(.object(["from": .string("2027-01-01T00:00:00Z")]))
        let past = try await source.search(.object(["to": .string("2025-01-01T00:00:00Z")]))
        guard future["items"].array.isEmpty, past["items"].array.isEmpty else { throw SourceFailure.corrupt }
        let sink = Sink()
        let task = Task { try await source.capture(.null, into: sink) }
        let initialDeadline = Date().addingTimeInterval(15)
        while await sink.count == 0 && Date() < initialDeadline { try await Task.sleep(for: .milliseconds(25)) }
        guard await sink.count == 1 else { throw SourceFailure.corrupt }
        if CommandLine.arguments.contains("PHO-IDLE") {
            let cpuStart = clock_gettime_nsec_np(CLOCK_PROCESS_CPUTIME_ID)
            let wallStart = Date()
            try await Task.sleep(for: .seconds(300))
            let cpu = Double(clock_gettime_nsec_np(CLOCK_PROCESS_CPUTIME_ID) - cpuStart) / 1e9
            let percent = cpu / Date().timeIntervalSince(wallStart) * 100
            print("idle_cpu_percent=\(percent)")
            guard percent < 2 else { throw SourceFailure.capabilityUnproven }
            guard await sink.count == 1 else { throw SourceFailure.corrupt }
            for handle in handles {
                do { _ = try await source.readBlob(handle, offset: 0, length: 1); throw SourceFailure.corrupt } catch SourceFailure.unavailable {}
            }
        }
        if CommandLine.arguments.contains("PHO-CAPTURE") {
            try await source.replaceFixture([])
            let changeDeadline = Date().addingTimeInterval(45)
            while await sink.count < 2 && Date() < changeDeadline { try await Task.sleep(for: .milliseconds(25)) }
            print("capture_checkpoints=\(await sink.count)")
            guard await sink.count == 2, await sink.rows.isEmpty else { throw SourceFailure.corrupt }
        }
        task.cancel()
        _ = try? await task.value
        print("fixture_checks=passed")
    }
}
