import Foundation

public struct MediaWork: Sendable {
    public let digest: String
    public let checkpoint: @Sendable (Duration) async throws -> Void
    public let interruption: @Sendable (@escaping @Sendable () -> Void) -> (@Sendable () -> Void)
    public let trace: @Sendable (String) -> Void
    @TaskLocal public static var current: MediaWork?
    public init(digest: String, checkpoint: @escaping @Sendable (Duration) async throws -> Void, interruption: @escaping @Sendable (@escaping @Sendable () -> Void) -> (@Sendable () -> Void), trace: @escaping @Sendable (String) -> Void) {
        self.digest = digest
        self.checkpoint = checkpoint
        self.interruption = interruption
        self.trace = trace
    }
    public static func yield(_ quiet: Duration = .milliseconds(250)) async throws {
        try Task.checkCancellation()
        try await current?.checkpoint(quiet)
    }
    static func validating<T: Sendable>(_ input: URL, operation: @Sendable () async throws -> T) async throws -> T {
        let stamp = try MediaSourceStamp(input, digest: current?.digest ?? "")
        guard let parent = current else {
            let result = try await operation()
            try stamp.validate(input)
            return result
        }
        let nested = MediaWork(digest: parent.digest, checkpoint: { quiet in
            try await parent.checkpoint(quiet)
            try stamp.validate(input)
        }, interruption: parent.interruption, trace: parent.trace)
        return try await $current.withValue(nested) {
            let result = try await operation()
            try stamp.validate(input)
            return result
        }
    }
    static func stage(_ name: String) async throws {
        try await yield()
        current?.trace(name)
    }
}
