public struct CaptureRetry: Sendable {
    private var seconds: Int64 = 1
    public init() {}
    public mutating func delay(after uptime: Duration, permissionMissing: Bool) -> Duration {
        if permissionMissing || uptime >= .seconds(30) {
            seconds = 1
            return .seconds(1)
        }
        seconds = min(30, seconds * 2)
        return .seconds(seconds)
    }
}
