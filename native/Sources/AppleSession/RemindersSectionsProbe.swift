#if LAB
import AppKit
import ApplicationServices
import RemindersSource
import RemindersStore
import SourceSupport

@MainActor
public enum RemindersSectionsProbe {
    public static func run() async -> JSON {
        let tag = "[ab-test] " + UUID().uuidString
        var assertions: [String: JSON] = [:]
        var created = false
        var reason = ""
        var listID = ""
        do {
            guard AXIsProcessTrusted() else { throw SourceFailure.permissionMissing }
            let url = URL(fileURLWithPath: "/System/Applications/Reminders.app")
            _ = try await NSWorkspace.shared.openApplication(at: url, configuration: NSWorkspace.OpenConfiguration())
            try await Task.sleep(for: .milliseconds(400))
            try RemindersSectionsUI.press("File")
            try RemindersSectionsUI.press("New List…")
            try await Task.sleep(for: .milliseconds(200))
            try RemindersSectionsUI.fill(tag)
            try RemindersSectionsUI.press("OK")
            created = true
            try await Task.sleep(for: .milliseconds(300))
            try await RemindersSectionsUI.createSection("[ab-test] First")
            try await RemindersSectionsUI.createSection("[ab-test] Second")
            assertions["ui_sections_created"] = .bool(true)
            var matched = false
            for path in try RemindersStore.paths() {
                let snapshot = try RemindersStore(path: path).read()
                guard let list = snapshot.lists.first(where: { $0["ZNAME"]?.string == tag }) else { continue }
                listID = list["ZIDENTIFIER"]?.string ?? ""
                let sections = snapshot.sections.filter { $0["ZLIST"] == list["Z_PK"] }
                assertions["store_section_count"] = .bool(sections.count == 2)
                let order = Data(base64Encoded: list["ZSECTIONIDSORDERINGASDATA"]?.string ?? "")
                let membership = Data(base64Encoded: list["ZMEMBERSHIPSOFREMINDERSINSECTIONSASDATA"]?.string ?? "")
                assertions["store_decoder"] = .bool(StoreSections.decode(order: order, membership: membership) != nil)
                matched = true
            }
            assertions["store_owned_list"] = .bool(matched)
            try RemindersSectionsUI.press("File")
            try RemindersSectionsUI.press("New Reminder")
            try RemindersSectionsUI.fill("[ab-test] item")
            try RemindersSectionsUI.key(36)
            try RemindersSectionsUI.move("[ab-test] item", to: "[ab-test] First")
            try RemindersSectionsUI.move("[ab-test] item", to: "[ab-test] Second")
            try await Task.sleep(for: .milliseconds(500))
            var membershipVerified = false
            for path in try RemindersStore.paths() {
                let snapshot = try RemindersStore(path: path).read()
                guard let list = snapshot.lists.first(where: { $0["ZNAME"]?.string == tag }),
                      let item = snapshot.reminders.first(where: { $0["ZLIST"] == list["Z_PK"] && $0["ZTITLE"]?.string == "[ab-test] item" }),
                      let section = snapshot.sections.first(where: { $0["ZLIST"] == list["Z_PK"] && $0["ZDISPLAYNAME"]?.string == "[ab-test] Second" }),
                      let decoded = StoreSections.decode(order: Data(base64Encoded: list["ZSECTIONIDSORDERINGASDATA"]?.string ?? ""), membership: Data(base64Encoded: list["ZMEMBERSHIPSOFREMINDERSINSECTIONSASDATA"]?.string ?? "")),
                      let itemID = UUID(uuidString: item["ZIDENTIFIER"]?.string ?? ""),
                      let sectionID = UUID(uuidString: section["ZIDENTIFIER"]?.string ?? "") else { continue }
                membershipVerified = decoded.membership[itemID] == sectionID
            }
            assertions["store_move_membership"] = .bool(membershipVerified)
            assertions["ui_move_completed"] = .bool(true)
        } catch {
            reason = String(describing: error)
            assertions["native_sections_effect"] = .bool(false)
        }
        if created {
            do {
                guard try RemindersStore.paths().contains(where: { path in
                    try RemindersStore(path: path).read().lists.contains { $0["ZNAME"]?.string == tag && (listID.isEmpty || $0["ZIDENTIFIER"]?.string == listID) }
                }) else { throw SourceFailure.conflict }
                try RemindersSectionsUI.selectList(tag)
                try RemindersSectionsUI.press("Edit")
                try RemindersSectionsUI.press("Delete List")
                try RemindersSectionsUI.press("Delete")
                try await Task.sleep(for: .milliseconds(300))
                assertions["owned_list_cleanup"] = .bool(try RemindersStore.paths().allSatisfy { path in
                    try !RemindersStore(path: path).read().lists.contains { $0["ZNAME"]?.string == tag }
                })
            } catch { assertions["owned_list_cleanup"] = .bool(false) }
        }
        return .object(["reason": .string(reason), "assertions": .object(assertions), "passed": .bool(!assertions.isEmpty && assertions.values.allSatisfy { $0.bool })])
    }
}
#endif
