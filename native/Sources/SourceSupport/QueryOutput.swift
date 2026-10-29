public struct QueryBlob: Sendable {
    public let digest: String
    public let size: UInt64
    public let handle: String
    public init(digest: String, size: UInt64, handle: String) {
        self.digest = digest; self.size = size; self.handle = handle
    }
    public var reference: JSON { .object(["digest": .string(digest), "size": .integer(size)]) }
    public var wireReference: JSON { .object(["digest": .string(digest), "size": .integer(size), "handle": .string(handle)]) }
}

public struct QueryOutput: Sendable {
    public let result: JSON
    public let blobs: [QueryBlob]
    public init(result: JSON, blobs: [QueryBlob] = []) {
        self.result = result; self.blobs = blobs
    }
}
