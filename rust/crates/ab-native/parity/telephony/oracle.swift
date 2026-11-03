import Foundation
import SourceSupport
import CallsSource
import VoicemailSource

func context(_ root: URL) throws -> JSON { try JSONDecoder().decode(JSON.self, from: Data(contentsOf: root.appendingPathComponent("context.json"))) }
func output(_ value: JSON) throws {
    let encoder=JSONEncoder();encoder.outputFormatting=[.sortedKeys,.withoutEscapingSlashes]
    try FileHandle.standardOutput.write(contentsOf: encoder.encode(value)+Data([10]))
}
actor Sink: ObservationSink {
    let root: URL
    let coverage: JSON
    init(_ root: URL, _ coverage: JSON) {self.root=root;self.coverage=coverage}
    func sourceState(source: String,state: String,coverage: JSON,capabilities: [String],reason: String) async throws {throw SourceFailure.unsupported}
    func emit(source: String,generation: String,observations: [JSON],scanID: String,scanStartedAt: String,scope: [String],completionEvidence: String,blobHandles: [String:String],live: Bool) async throws {throw SourceFailure.unsupported}
    func emitCheckpoint(source: String,generation: String,checkpoint: String,observations: [JSON],scanID: String,scanStartedAt: String,scope: [String],completionEvidence: String,blobHandles: [String:String],live: Bool) async throws {
        let ctx=try context(root)
        let rows=observations.map {row -> JSON in var o=row.object;o["observed_at"]=ctx["observed_at"];return .object(o)}
        try output(.object(["source":.string(source),"generation":.string(generation),"checkpoint":ctx["checkpoint"],"scan_id":ctx["scan_id"],"scan_started_at":ctx["observed_at"],"scope":.array(scope.map(JSON.string)),"completion_evidence":.string(completionEvidence),"coverage":coverage,"observations":.array(rows),"live":.bool(live)]))
    }
}
@main struct Oracle {
    @MainActor static func main() async throws {
        let kind=CommandLine.arguments[1],root=URL(fileURLWithPath:CommandLine.arguments[2])
        let calls=CallsSource(path:root.appendingPathComponent("store.db").path)
        let voicemail=VoicemailSource(path:root.appendingPathComponent("store.db").path)
        let readiness = kind == "calls" ? await calls.open() : await voicemail.open()
        let sink=Sink(root,readiness.coverage)
        var index=0
        let commands=AsyncStream<String> {continuation in Task.detached {while let line=readLine(){continuation.yield(line)};continuation.finish()}}
        for await command in commands {
            let limit=Int(command) ?? 100000
            do {
                if kind == "calls" {try await calls.scan(into:sink,live:index>0,maxRows:limit)}
                else {try await voicemail.scan(into:sink,live:index>0,maxRows:limit)}
            }catch{try output(.object(["kind":.string("failed_scan")]))}
            index += 1
        }
    }
}
