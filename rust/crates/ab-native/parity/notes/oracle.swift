import Foundation
import SourceSupport
import NotesSource
import UniformTypeIdentifiers
import MediaDerivatives

func context(_ root:URL)throws->JSON{try JSONDecoder().decode(JSON.self,from:Data(contentsOf:root.appendingPathComponent("context.json")))}
func output(_ value:JSON)throws{
    let encoder=JSONEncoder();encoder.outputFormatting=[.sortedKeys,.withoutEscapingSlashes]
    try FileHandle.standardOutput.write(contentsOf:encoder.encode(value)+Data([10]))
}
actor Sink:ObservationSink{
    let root:URL
    var ordinal=0
    var scans:[String:String]=[:]
    init(_ root:URL){self.root=root}
    func sourceState(source:String,state:String,coverage:JSON,capabilities:[String],reason:String)async throws{throw SourceFailure.unsupported}
    func emit(source:String,generation:String,observations:[JSON],scanID:String,scanStartedAt:String,scope:[String],completionEvidence:String,blobHandles:[String:String],live:Bool)async throws{
        let ctx=try context(root)
        var normalized=scanID
        if !scope.isEmpty{scans[scanID]=ctx["scan_id"].string;normalized=ctx["scan_id"].string}
        else{for suffix in ["-hydrate","-unavailable"] where scanID.hasSuffix(suffix){normalized=(scans[String(scanID.dropLast(suffix.count))] ?? "unknown")+suffix}}
        let rows=observations.map{row->JSON in var value=row.object;value["observed_at"]=ctx["observed_at"];return .object(value)}
        var frame:[String:JSON]=["source":.string(source),"generation":.string(generation),"checkpoint":.string(ctx["checkpoint"].string+"-"+String(ordinal)),"scan_id":.string(normalized),"scan_started_at":ctx["observed_at"],"scope":.array(scope.map(JSON.string)),"completion_evidence":.string(completionEvidence),"coverage":ctx["coverage"],"observations":.array(rows),"live":.bool(live)]
        if !blobHandles.isEmpty{frame["blob_handles"] = .object(blobHandles.mapValues(JSON.string))}
        try output(.object(frame));ordinal+=1
    }
}
@main struct Oracle{
    @MainActor static func main()async throws{
        let root=URL(fileURLWithPath:CommandLine.arguments[1]),home=root.appendingPathComponent("swift-home")
        let source=try NotesSource(home:home,fixtureDatabase:root.appendingPathComponent("NoteStore.sqlite")),sink=Sink(root)
        try output(.object(["kind":.string("ready")]))
        let commands=AsyncStream<String>{continuation in Task.detached{while let line=readLine(){continuation.yield(line)};continuation.finish()}}
        for await line in commands{
            if line=="facts"{
                let ctx=try context(root)
                var mime:[String:JSON]=[:],supports:[String:JSON]=[:]
                for value in ctx["fact_types"].array{mime[value.string]=UTType(value.string)?.preferredMIMEType.map(JSON.string) ?? .null}
                for pair in ctx["fact_pairs"].array{
                    let uti=pair["uti"].string,ext=pair["extension"].string
                    supports[uti+"|"+ext] = .bool(MediaDerivatives.supports(URL(fileURLWithPath:"fixture."+ext),uti:uti))
                }
                try output(.object(["facts":.object(["mime":.object(mime),"supports":.object(supports)])]));continue
            }
            let parts=line.split(separator:" ").map(String.init)
            if parts.first=="blob",parts.count==4{
                do{try output(source.readBlob(parts[1],offset:Int(parts[2]) ?? -1,length:Int(parts[3]) ?? 0))}catch{try output(.object(["kind":.string("failed_blob")]))}
                continue
            }
            if line=="hold"{try Data().write(to:home.appendingPathComponent("lab-notes-hold-hydration"))}
            else if line=="release"{try? FileManager.default.removeItem(at:home.appendingPathComponent("lab-notes-hold-hydration"))}
            else if line=="scan"{
                do{let ctx=try context(root);try await source.reconcile(into:sink,assetPaths:Set(ctx["asset_paths"].array.map(\.string)))}
                catch{try output(.object(["kind":.string("failed_scan")]));continue}
            }else{throw SourceFailure.invalid}
            if !FileManager.default.fileExists(atPath:home.appendingPathComponent("lab-notes-hold-hydration").path){try await source.waitForHydration()}
            try output(.object(["kind":.string("done"),"changed_notes":.number(Double(source.lastChangedNotes)),"bootstrap_scans":.number(Double(source.bootstrapScans))]))
        }
    }
}
