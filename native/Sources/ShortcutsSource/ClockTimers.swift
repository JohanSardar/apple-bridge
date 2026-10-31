import Foundation
import ClockNative
import SourceSupport

public enum ClockTimers {
    public static func exchange(_ request: Data) throws -> Data {
        guard request.count <= 32768 else { throw SourceFailure.invalid }
        let result = ABClockTimersExchange(request)
        guard result.count <= 32768, let value = try JSONSerialization.jsonObject(with: result) as? [String: Any], value["error"] == nil else { throw SourceFailure.unavailable }
        return result
    }
}
