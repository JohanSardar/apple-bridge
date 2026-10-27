import Foundation

public struct MessageActionWindow: Sendable {
    public static let editSeconds: TimeInterval = 900
    public static let unsendSeconds: TimeInterval = 120
    public static let editLimit = 5
    public let sentAt: Date
    public let editUntil: Date?
    public let unsendUntil: Date
    public init?(service: String, isFromMe: Bool, isMessage: Bool, sentAt: Date?, edits: Int) {
        guard service == "iMessage", isFromMe, isMessage, let sentAt else { return nil }
        self.sentAt = sentAt
        editUntil = edits < Self.editLimit ? sentAt.addingTimeInterval(Self.editSeconds) : nil
        unsendUntil = sentAt.addingTimeInterval(Self.unsendSeconds)
    }
    public func allows(edit: Bool, at now: Date = Date()) -> Bool {
        guard let until = edit ? editUntil : unsendUntil else { return false }
        return now >= sentAt && now < until
    }
}
