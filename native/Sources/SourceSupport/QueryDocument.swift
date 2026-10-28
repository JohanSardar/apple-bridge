import Foundation
import Darwin

public func queryDocument(_ args: JSON) throws -> Data {
    let encoded = args["data_base64"].string
    let path = args["file"].string
    guard encoded.isEmpty != path.isEmpty else { throw SourceFailure.invalid }
    if !encoded.isEmpty {
        guard let data = Data(base64Encoded: encoded), !data.isEmpty, data.count <= 8 << 20 else { throw SourceFailure.invalid }
        return data
    }
    guard path.hasPrefix("/") else { throw SourceFailure.invalid }
    let descriptor = open(path, O_RDONLY | O_NONBLOCK)
    guard descriptor >= 0 else { throw SourceFailure.unavailable }
    let handle = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
    var info = stat()
    guard fstat(descriptor, &info) == 0, (info.st_mode & S_IFMT) == S_IFREG,
          info.st_size > 0, info.st_size <= 8 << 20 else {
        try? handle.close()
        throw SourceFailure.invalid
    }
    defer { try? handle.close() }
    guard let data = try handle.read(upToCount: (8 << 20) + 1), !data.isEmpty, data.count <= 8 << 20 else { throw SourceFailure.invalid }
    return data
}
