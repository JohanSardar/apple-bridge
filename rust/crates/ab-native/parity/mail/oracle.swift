import Foundation
import SourceSupport
import MailSource

func output(_ value:JSON)throws{
    let encoder=JSONEncoder();encoder.outputFormatting=[.sortedKeys,.withoutEscapingSlashes]
    try FileHandle.standardOutput.write(contentsOf:encoder.encode(value)+Data([10]))
}
@main struct Oracle{
    static func main()async throws{
        let root=URL(fileURLWithPath:CommandLine.arguments[1])
        let source=MailSource(path:root.appendingPathComponent("store.db").path,script:{_ in throw SourceFailure.unsupported})
        var previous:[JSON]=[]
        let commands=AsyncStream<String>{continuation in Task.detached{while let line=readLine(){continuation.yield(line)};continuation.finish()}}
        for await command in commands{
            if command.hasPrefix("body "){
                do {let args=try JSONDecoder().decode(JSON.self,from:Data(command.dropFirst(5).utf8));try output(MailBody.read(args,root:root.appendingPathComponent("bodies")))}
                catch{try output(.object(["kind":.string("failed_body")]))}
                continue
            }
            do{
                let ctx=try JSONDecoder().decode(JSON.self,from:Data(contentsOf:root.appendingPathComponent("context.json")))
                let (generation,observations,complete)=try await source.snapshot(maxRows:Int(command) ?? 100000)
                let material=observations.map{row -> JSON in var fields=row.object;fields.removeValue(forKey:"observed_at");return .object(fields)}
                if material==previous{try output(.object(["kind":.string("idle")]));continue}
                let rows=observations.map{row -> JSON in var fields=row.object;fields["observed_at"]=ctx["observed_at"];return .object(fields)}
                try output(.object(["source":.string("mail"),"generation":.string(generation),"checkpoint":ctx["checkpoint"],"scan_id":ctx["scan_id"],"scan_started_at":ctx["observed_at"],"scope":.array(["mail.account","mail.mailbox","mail.message"].map(JSON.string)),"completion_evidence":.string(complete ? "complete":"partial"),"coverage":MailSource.coverage,"observations":.array(rows),"live":.bool(!previous.isEmpty)]))
                previous=material
            }catch{try output(.object(["kind":.string("failed_scan")]))}
        }
    }
}
