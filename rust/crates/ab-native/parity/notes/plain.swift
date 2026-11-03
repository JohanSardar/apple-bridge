import Foundation
import Decoders
import SourceSupport
for file in try FileManager.default.contentsOfDirectory(at:URL(fileURLWithPath:CommandLine.arguments[1]),includingPropertiesForKeys:nil).filter({$0.pathExtension=="gz"}).sorted(by:{$0.lastPathComponent<$1.lastPathComponent}){
    var value:[String:JSON]=["fixture":.string(file.lastPathComponent)]
    do{value["text"] = .string(try NotesProto.plainText(Data(contentsOf:file)))}catch{value["error"] = .bool(true)}
    let encoder=JSONEncoder();encoder.outputFormatting=[.sortedKeys,.withoutEscapingSlashes]
    try FileHandle.standardOutput.write(contentsOf:encoder.encode(JSON.object(value))+Data([10]))
}
