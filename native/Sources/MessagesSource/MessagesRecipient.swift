import Foundation
import ApplicationServices
import SourceSupport

public enum MessagesRecipient {
    public static func recipientMatches(_ description: String, handle: String) -> Bool {
        let expected = MessagesProbes.normalize(handle)
        guard !expected.isEmpty else { return false }
        if expected.contains("@") {
            let pattern = #"[A-Za-z0-9.!#$%&'*+/=?^_`{|}~-]+@[A-Za-z0-9.-]+"#
            guard let expression = try? NSRegularExpression(pattern: pattern) else { return false }
            let range = NSRange(description.startIndex..., in: description)
            let addresses = expression.matches(in: description, range: range).compactMap { Range($0.range, in: description).map { String(description[$0]).lowercased() } }
            return addresses == [expected]
        }
        return description.filter(\.isNumber) == expected.filter(\.isNumber)
    }
}
