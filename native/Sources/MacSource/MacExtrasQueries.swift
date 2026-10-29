import Foundation
import SourceSupport

public enum MacExtrasQueries {
    public static func run(_ op: String, args: JSON) async throws -> JSON {
        if op == "mac.recognize_song" {
            guard let bytes = Data(base64Encoded: args["audio_blob"].string), !bytes.isEmpty, bytes.count <= 8 * 1024 * 1024 else { throw SourceFailure.invalid }
            return .object(["limited": .bool(true), "reason": .string("ShazamKit entitlement and fixture qualification required")])
        }
        guard args == .object([:]) else { throw SourceFailure.invalid }
        if op == "mac.time_machine_status" {
            return try await Task.detached { () throws -> JSON in
                let process = Process()
                process.executableURL = URL(fileURLWithPath: "/usr/bin/tmutil")
                process.arguments = ["status"]
                let pipe = Pipe()
                process.standardOutput = pipe
                process.standardError = FileHandle.nullDevice
                try process.run()
                let data = pipe.fileHandleForReading.readDataToEndOfFile()
                process.waitUntilExit()
                guard process.terminationStatus == 0, let text = String(data: data, encoding: .utf8) else { throw SourceFailure.unavailable }
                return .object(["status": .string(text)])
            }.value
        }
        return .object(["limited": .bool(true), "reason": .string("Device discovery and route access require real qualification; no authorization requested")])
    }
}
