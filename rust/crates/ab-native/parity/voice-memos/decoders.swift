import Foundation
import SourceSupport
import VoiceMemosSource

let input = try JSONDecoder().decode([[String:String]].self,from:Data(contentsOf:URL(fileURLWithPath:CommandLine.arguments[1])))
let root=FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
try FileManager.default.createDirectory(at:root,withIntermediateDirectories:true)
defer {try? FileManager.default.removeItem(at:root)}
for vector in input {
    let data=Data(base64Encoded:vector["input"]!)!
    var row: [String:JSON] = ["name":.string(vector["name"]!),"input":.string(vector["input"]!),"encoding":.string(vector["encoding"]!)]
    do {
        let result: [JSON]
        if vector["encoding"] == "mp4" {
            let path=root.appendingPathComponent("fixture.m4a");try data.write(to:path)
            result=try VoiceMemoTranscript.read(path)
        } else {result=try VoiceMemoTranscript.decode(data)}
        row["result"] = .array(result)
    } catch {row["error"] = .bool(true)}
    let encoder=JSONEncoder();encoder.outputFormatting=[.sortedKeys,.withoutEscapingSlashes]
    try FileHandle.standardOutput.write(contentsOf:encoder.encode(JSON.object(row))+Data([10]))
}
