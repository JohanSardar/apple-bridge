import Foundation
import SourceSupport
import VoiceMemosSource

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
        var frame: [String:JSON] = ["source":.string(source),"generation":.string(generation),"checkpoint":ctx["checkpoint"],"scan_id":ctx["scan_id"],"scan_started_at":ctx["observed_at"],"scope":.array(scope.map(JSON.string)),"completion_evidence":.string(completionEvidence),"coverage":coverage,"observations":.array(rows),"live":.bool(live)]
        if !blobHandles.isEmpty { frame["blob_handles"] = .object(blobHandles.mapValues(JSON.string)) }
        try output(.object(frame))
    }
}
@main struct Oracle {
    @MainActor static func main() async throws {
        let root=URL(fileURLWithPath:CommandLine.arguments[1])
        let source=VoiceMemosSource(path:root.appendingPathComponent("store.db").path)
        let readiness=await source.open()
        let sink=Sink(root,readiness.coverage)
        var index=0
        let commands=AsyncStream<String> {continuation in Task.detached {while let line=readLine(){continuation.yield(line)};continuation.finish()}}
        for await command in commands {
            let parts=command.split(separator:" ").map(String.init)
            if parts.first == "blob", parts.count == 4 {
                do {try output(source.readBlob(parts[1],offset:Int64(parts[2]) ?? -1,length:Int(parts[3]) ?? 0))}
                catch {try output(.object(["kind":.string("failed_blob")]))}
                continue
            }
            let limit=Int(command) ?? 100000
            do {try await source.scan(into:sink,live:index>0,maxRows:limit)}
            catch {try output(.object(["kind":.string("failed_scan")]))}
            index += 1
        }
    }
}
