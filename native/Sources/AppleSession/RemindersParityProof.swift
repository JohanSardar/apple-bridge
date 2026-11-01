#if LAB
import Foundation
import AppKit
import ApplicationServices
import Carbon
import EventKit
import RemindersStore
import SourceSupport
import MessagesSource

@MainActor
enum RemindersParityProof {
    static func run() async throws -> JSON {
        var checks: [String: JSON] = [:]
        var reasons: [String: JSON] = [:]
        var diagnostics: [String: JSON] = [:]
        let target = NSAppleEventDescriptor(bundleIdentifier: "com.apple.reminders")
        let permission = AEDeterminePermissionToAutomateTarget(target.aeDesc, AEEventClass(kAECoreSuite), AEEventID(kAEGetData), false)
        checks["automation_granted"] = .bool(permission == noErr)
        checks["accessibility_granted"] = .bool(AXIsProcessTrusted())
        checks["reminders_granted"] = .bool(EKEventStore.authorizationStatus(for: .reminder) == .fullAccess)
        guard checks.values.allSatisfy(\.bool) else {
            return .object(["passed": .bool(false), "checks": .object(checks), "reason": .string(permission != noErr ? "Reminders Automation is unavailable to the signed Helper; grant it in System Settings before retrying." : "The signed Helper needs Reminders and Accessibility access in System Settings.")])
        }
        let store = EKEventStore()
        let run = UUID().uuidString
        let title = "[ab-test " + run + "]"
        let list = EKCalendar(for: .reminder, eventStore: store)
        guard let source = store.defaultCalendarForNewReminders()?.source else { throw SourceFailure.unavailable }
        diagnostics["fixture_source_type"] = .number(Double(source.sourceType.rawValue))
        diagnostics["fixture_icloud_source"] = .number(source.title.caseInsensitiveCompare("iCloud") == .orderedSame ? 1 : 0)
        list.source = source
        list.title = title
        try store.saveCalendar(list, commit: true)
        let listID = list.calendarIdentifier
        defer {
            store.reset()
            if let owned = store.calendar(withIdentifier: listID), owned.title == title { try? store.removeCalendar(owned, commit: true) }
        }
        let ledger = PrivateReport.directory.appendingPathComponent("reminders-parity-cleanup/" + run + ".json")
        try FileManager.default.createDirectory(at: ledger.deletingLastPathComponent(), withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        try PrivateReport.write(["list_id": listID, "title": title, "smart_title": title + " smart", "cleanup_state": "pending"], to: ledger)
        let first = EKReminder(eventStore: store), second = EKReminder(eventStore: store)
        first.calendar = list; first.title = title + " parent"
        second.calendar = list; second.title = title + " child"
        let smartTitle = title + " smart"
        var smartAttempted = false
        var fixtureUI: RemindersFixtureUI?
        var stage = "fixture"
        do {
            try store.save(first, commit: true)
            try store.save(second, commit: true)
            let firstID = first.calendarItemIdentifier, secondID = second.calendarItemIdentifier
            let sessions = try RemindersStore.paths().map { try RemindersStoreSession(path: $0) }
            var entityNames: [Int64: String] = [:]
            @MainActor func observe(_ predicate: (TypedStoreSnapshot, StoreListRow, StoreReminderRow, StoreReminderRow) -> Bool) async throws -> Bool {
                let deadline = ContinuousClock.now.advanced(by: .seconds(30))
                repeat {
                    for session in sessions {
                        let read = try await session.read()
                        let snapshot = read.snapshot
                        entityNames = read.entities
                        diagnostics[stage + "_parent_matches"] = .number(max(diagnostics[stage + "_parent_matches"]?.number ?? 0, Double(snapshot.reminders.values.filter { $0.calendarItemIdentifier == firstID }.count)))
                        if let parent = snapshot.reminders.values.first(where: { $0.calendarItemIdentifier == firstID }) {
                            diagnostics[stage + "_store_flag"] = .number(parent.flagged.map { $0 ? 1 : 0 } ?? -1)
                        }
                        if let row = snapshot.lists.values.first(where: { $0.identity.uuid?.uuidString == listID || $0.identity.textIdentifier == listID }),
                           let parent = snapshot.reminders.values.first(where: { $0.calendarItemIdentifier == firstID }),
                           let child = snapshot.reminders.values.first(where: { $0.calendarItemIdentifier == secondID }), predicate(snapshot, row, parent, child) { return true }
                    }
                    try await Task.sleep(for: .milliseconds(100))
                } while ContinuousClock.now < deadline
                for path in try RemindersStore.paths() {
                    let snapshot = try await RemindersStoreSession(path: path).read().snapshot
                    if let parent = snapshot.reminders.values.first(where: { $0.calendarItemIdentifier == firstID }) {
                        diagnostics[stage + "_full_store_flag"] = .number(parent.flagged.map { $0 ? 1 : 0 } ?? -1)
                    }
                }
                return false
            }
            @discardableResult
            func script(_ body: String) throws -> NSAppleEventDescriptor {
                var error: NSDictionary?
                let source = "with timeout of 30 seconds\ntell application \"Reminders\"\nset matches to every list whose name is \"" + title + "\"\nif (count matches) is not 1 then error number -2700\nset fixtureList to item 1 of matches\n" + body + "\nend tell\nend timeout"
                guard let command = NSAppleScript(source: source) else { throw SourceFailure.invalid }
                let result = command.executeAndReturnError(&error)
                if let error {
                    diagnostics[stage + "_script_error"] = .number((error[NSAppleScript.errorNumber] as? NSNumber)?.doubleValue ?? 0)
                    if let range = error[NSAppleScript.errorRange] as? NSValue { diagnostics[stage + "_script_error_offset"] = .number(Double(range.rangeValue.location)) }
                    throw SourceFailure.unavailable
                }
                return result
            }
            stage = "fixture_visibility"
            var visible = false
            let visibilityDeadline = ContinuousClock.now.advanced(by: .seconds(10))
            repeat {
                if let result = try? script("return count (every reminder of fixtureList)"), result.int32Value == 2 {
                    visible = true
                    diagnostics.removeValue(forKey: stage + "_script_error")
                    break
                }
                try await Task.sleep(for: .milliseconds(100))
            } while ContinuousClock.now < visibilityDeadline
            checks["fixture_visible"] = .bool(visible)
            guard visible else { throw SourceFailure.unavailable }
            stage = "store_visibility"
            checks["store_fixture_visible"] = .bool(try await observe { _, _, _, _ in true })
            stage = "flag"
            try script("set fixtureItems to every reminder of fixtureList whose name is \"" + first.title + "\"\nif (count fixtureItems) is not 1 then error number -2701\nignoring application responses\nset flagged of item 1 of fixtureItems to true\nend ignoring")
            diagnostics["flag_script_readback"] = .number(Double((try? script("return flagged of (first reminder of fixtureList whose name is \"" + first.title + "\")").int32Value) ?? -2))
            checks["flag_readback"] = .bool(try await observe { _, _, parent, _ in parent.flagged == true })
            try script("set fixtureItems to every reminder of fixtureList whose name is \"" + first.title + "\"\nif (count fixtureItems) is not 1 then error number -2701\nignoring application responses\nset flagged of item 1 of fixtureItems to false\nend ignoring")
            stage = "flag_clear"
            diagnostics["flag_clear_script_readback"] = .number(Double((try? script("return flagged of (first reminder of fixtureList whose name is \"" + first.title + "\")").int32Value) ?? -2))
            checks["flag_clear_readback"] = .bool(try await observe { _, _, parent, _ in parent.flagged == false })
            stage = "list_icon"
            try script("ignoring application responses\nset emblem of fixtureList to \"star\"\nend ignoring")
            checks["list_icon_readback"] = .bool(try await observe { _, row, _, _ in row.emblem == "star" })
            stage = "navigation"
            try script("reopen\nshow fixtureList\nactivate")
            try await Task.sleep(for: .seconds(1))
            guard let app = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.reminders").first else { throw SourceFailure.unavailable }
            let ui = RemindersFixtureUI(app: app, title: title)
            fixtureUI = ui
            try await ui.prepare()
            for (key, value) in ui.accessibilityDiagnostics { diagnostics["ui_" + key] = .number(value) }
            try ui.requireReady()
            try script("show fixtureList\nactivate")
            try await Task.sleep(for: .milliseconds(500))
            let expected = ["File", "Edit", "View", "New Section", "Indent Reminders", "Outdent Reminders", "Add Tags…", "Add Image", "New List…", "Show List Info", "Sort By", "Manual", "Add Image…", "Choose File…"]
            diagnostics["ui_windows"] = .number(Double(ui.elements(ui.root, kAXWindowsAttribute).count))
            checks["owned_list_visible"] = .bool(!ui.find(title).isEmpty)
            for (key, value) in ui.searchDiagnostics { diagnostics["selection_" + key] = .number(value) }
            for name in expected { checks["control_" + name] = .bool(!ui.find(name).isEmpty) }
            let tag = "abtest" + UUID().uuidString.replacingOccurrences(of: "-", with: "").lowercased()
            @MainActor func attempt(_ name: String, _ action: @MainActor () async throws -> Void) async {
                stage = name
                ui.selectionDiagnostics = [:]
                do {
                    try await ui.activate()
                    try await action()
                }
                catch { reasons[name] = .string("The " + name + " action could not be verified on the test list: " + String(describing: error)) }
                for (key, value) in ui.selectionDiagnostics { diagnostics[name + "_" + key] = .number(value) }
            }
            await attempt("images") {
                try script("show (first reminder of fixtureList whose name is \"" + second.title + "\")\nactivate")
                try await Task.sleep(for: .milliseconds(300))
                _ = try await ui.selectReminder(second.title)
                let pasteboard = NSPasteboard.general
                let saved = (pasteboard.pasteboardItems ?? []).map { item in item.types.compactMap { type in item.data(forType: type).map { (type, $0) } } }
                guard let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 2, pixelsHigh: 2, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 8, bitsPerPixel: 32) else { throw SourceFailure.unavailable }
                for x in 0..<2 { for y in 0..<2 { bitmap.setColor(.systemBlue, atX: x, y: y) } }
                guard let bytes = bitmap.representation(using: .png, properties: [:]) else { throw SourceFailure.unavailable }
                pasteboard.clearContents()
                guard pasteboard.setData(bytes, forType: .png) else { throw SourceFailure.unavailable }
                let installedChange = pasteboard.changeCount
                do {
                    defer {
                        if pasteboard.changeCount == installedChange {
                            pasteboard.clearContents()
                            let items = saved.map { values -> NSPasteboardItem in
                                let item = NSPasteboardItem()
                                for (type, bytes) in values { item.setData(bytes, forType: type) }
                                return item
                            }
                            pasteboard.writeObjects(items)
                        }
                    }
                    try ui.key(9, flags: .maskCommand)
                    checks["image_readback"] = .bool(try await observe { snapshot, _, _, child in
                        snapshot.objects.values.contains { object in
                            let owners = Set([object.reminder, object.reminder1, object.reminder2, object.reminder3, object.reminder4, object.reminder5].compactMap { $0 }.filter { $0 != 0 })
                            return entityNames[object.identity.entity] == "REMCDImageAttachment" && owners == [child.identity.key]
                        }
                    })
                }
            }
            await attempt("tags") {
                try script("show (first reminder of fixtureList whose name is \"" + first.title + "\")\nactivate")
                try await Task.sleep(for: .milliseconds(300))
                let row = try await ui.selectReminder(first.title)
                let fields = ui.find(first.title, role: kAXTextFieldRole, within: row) + ui.find(first.title, role: kAXTextAreaRole, within: row)
                ui.selectionDiagnostics["title_fields"] = Double(fields.count)
                if fields.count == 1 {
                    try ui.set(fields[0], text: first.title + " #" + tag)
                    try ui.key(36)
                    checks["tags_readback"] = .bool(try await observe { snapshot, _, parent, _ in StoreTags.names(for: parent.identity.key, snapshot: snapshot)?.contains(tag) == true })
                }
            }
            await attempt("subtasks") {
                try script("show fixtureList\nactivate")
                try await Task.sleep(for: .milliseconds(400))
                _ = try await ui.selectReminder(second.title)
                try await ui.menu("Edit", ["Indent Reminder", "Indent Reminders"])
                let indented = try await observe { _, _, parent, child in child.parent == parent.identity.key }
                checks["indent_readback"] = .bool(indented)
                guard indented else { throw SourceFailure.unavailable }
                _ = try await ui.selectReminder(second.title)
                try await ui.menu("Edit", ["Outdent Reminder", "Outdent Reminders"])
                checks["outdent_readback"] = .bool(try await observe { _, _, _, child in child.parent == nil || child.parent == 0 })
            }
            await attempt("reorder") {
                try script("show fixtureList\nactivate")
                try await Task.sleep(for: .milliseconds(300))
                try await ui.drag(second.title, to: first.title)
                checks["reorder_readback"] = .bool(try await observe { _, row, parent, child in
                    diagnostics["reorder_child_is_subtask"] = .number(child.parent == parent.identity.key ? 1 : 0)
                    diagnostics["reorder_parent_binary_uuid"] = .number(parent.identity.uuid == nil ? 0 : 1)
                    diagnostics["reorder_child_binary_uuid"] = .number(child.identity.uuid == nil ? 0 : 1)
                    guard case .supported(let positions) = StoreManualOrder.decode(row.orderingJSON), let parentIndex = StoreManualOrder.position(of: parent.identity, in: positions), let childIndex = StoreManualOrder.position(of: child.identity, in: positions) else { return false }
                    diagnostics["reorder_parent_position"] = .number(Double(parentIndex))
                    diagnostics["reorder_child_position"] = .number(Double(childIndex))
                    return childIndex < parentIndex
                })
            }
            await attempt("smart_list") {
                guard checks["tags_readback"]?.bool == true else { throw SourceFailure.unavailable }
                try await ui.menu("File", ["New List", "New List…", "New List..."])
                try await ui.setFocused(text: smartTitle)
                try ui.press("Standard", role: kAXPopUpButtonRole)
                try ui.press("Smart List", role: kAXMenuItemRole)
                try await Task.sleep(for: .milliseconds(300))
                let tagChoices = ui.find(tag, role: kAXCheckBoxRole)
                guard tagChoices.count == 1 else { throw SourceFailure.unsupported }
                try ui.press(tag, role: kAXCheckBoxRole)
                smartAttempted = true
                try ui.pressAny(["OK", "Add List", "Done"], role: kAXButtonRole)
                var found = false
                for _ in 0..<60 {
                    for session in sessions {
                        let snapshot = try await session.read().snapshot
                        found = found || snapshot.lists.values.contains { $0.name == smartTitle && $0.filter != nil && $0.smartType != nil }
                    }
                    if found { break }
                    try await Task.sleep(for: .milliseconds(100))
                }
                checks["smart_list_readback"] = .bool(found)
            }
        } catch {
            reasons[stage] = .string("The " + stage + " action could not be verified on the test list: " + String(describing: error))
        }
        if smartAttempted {
          do {
            var error: NSDictionary?
            _ = NSAppleScript(source: "with timeout of 5 seconds\ntell application \"Reminders\"\nset matches to every list whose name is \"" + smartTitle + "\"\nif (count matches) is 1 then delete item 1 of matches\nend tell\nend timeout")?.executeAndReturnError(&error)
            let sessions = try RemindersStore.paths().map { try RemindersStoreSession(path: $0) }
            var absent = false
            for _ in 0..<60 {
                var present = false
                for session in sessions {
                    let snapshot = try await session.read().snapshot
                    if snapshot.lists.values.contains(where: { $0.name == smartTitle }) { present = true }
                }
                if !present { absent = true; break }
                try await Task.sleep(for: .milliseconds(100))
            }
            if !absent, let app = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.reminders").first {
                let ui = RemindersFixtureUI(app: app, title: smartTitle)
                try await ui.menu("File", ["Delete List", "Delete List…"])
                try ui.press("Delete", role: kAXButtonRole)
                try await Task.sleep(for: .milliseconds(500))
                absent = true
                for session in sessions {
                    let snapshot = try await session.read().snapshot
                    if snapshot.lists.values.contains(where: { $0.name == smartTitle }) { absent = false }
                }
            }
            checks["owned_smart_list_cleanup"] = .bool(absent)
          } catch {
            checks["owned_smart_list_cleanup"] = .bool(false)
            reasons["cleanup"] = .string("The test smart list could not be removed; rerun cleanup for this run before testing again.")
          }
        }
        if let ui = fixtureUI {
          if ui.pendingNewListSheet { try? await ui.adoptNewListSheet() }
          if let sheet = ui.ownedSheet {
            if ui.sheets().contains(where: { CFEqual($0, sheet) }), ui.find("Cancel", role: kAXButtonRole, within: sheet).count == 1 {
                try? await ui.activate()
                try? ui.press("Cancel", role: kAXButtonRole)
                let deadline = ContinuousClock.now.advanced(by: .seconds(3))
                while ui.sheets().contains(where: { CFEqual($0, sheet) }), ContinuousClock.now < deadline {
                    try await Task.sleep(for: .milliseconds(100))
                }
            }
            checks["owned_sheet_cleanup"] = .bool(!ui.sheets().contains(where: { CFEqual($0, sheet) }))
          } else if ui.pendingNewListSheet {
            checks["owned_sheet_cleanup"] = .bool(false)
          }
        }
        store.reset()
        if let owned = store.calendar(withIdentifier: listID), owned.title == title { try store.removeCalendar(owned, commit: true) }
        store.reset()
        checks["owned_list_cleanup"] = .bool(store.calendar(withIdentifier: listID) == nil)
        try PrivateReport.write(["list_id": listID, "title": title, "smart_title": smartTitle, "cleanup_state": checks["owned_list_cleanup"]?.bool == true && (!smartAttempted || checks["owned_smart_list_cleanup"]?.bool == true) ? "verified_clean" : "pending"], to: ledger)
        let required = ["flag_readback", "flag_clear_readback", "list_icon_readback", "indent_readback", "outdent_readback", "tags_readback", "reorder_readback", "image_readback", "smart_list_readback", "owned_list_cleanup"]
        return .object(["passed": .bool(required.allSatisfy { checks[$0]?.bool == true } && checks["owned_sheet_cleanup"]?.bool != false && (!smartAttempted || checks["owned_smart_list_cleanup"]?.bool == true)), "checks": .object(checks.filter { !$0.key.hasPrefix("control_") }), "controls": .object(checks.filter { $0.key.hasPrefix("control_") }), "diagnostics": .object(diagnostics), "reasons": .object(reasons)])
    }
}

@MainActor
final class RemindersFixtureUI {
    let app: NSRunningApplication
    let root: AXUIElement
    let title: String
    var searchDiagnostics: [String: Double] = [:]
    var accessibilityDiagnostics: [String: Double] = [:]
    var ownedSheet: AXUIElement?
    var pendingNewListSheet = false
    var selectionDiagnostics: [String: Double] = [:]

    static func editableRole(_ role: String) -> Bool {
        [kAXTextFieldRole, kAXTextAreaRole].contains(role)
    }

    static func matches(_ labels: [String], name: String, title: String) -> Bool {
        labels.contains(name) || (name.hasPrefix(title) && labels.contains { $0.hasPrefix(name + ",") || $0.hasPrefix(name + "\n") || $0.hasPrefix(name + " #") })
    }

    init(app: NSRunningApplication, title: String) {
        self.app = app; self.title = title
        root = AXUIElementCreateApplication(app.processIdentifier)
        AXUIElementSetMessagingTimeout(root, 0.25)

    }

    enum ReadinessFailure: Error, CustomStringConvertible {
        case locationConsent

        var description: String { "Open Reminders and finish its Location consent screen before retrying. No Location permission was requested by this test." }
    }

    func requireReady() throws {
        if accessibilityDiagnostics["location_consent_blocked"] == 1 { throw ReadinessFailure.locationConsent }
    }

    func prepare() async throws {
        _ = app.activate()
        let recoveryURL = PrivateReport.directory.appendingPathComponent("reminders-owned-new-list-sheet.json")
        if FileManager.default.fileExists(atPath: recoveryURL.path) {
            let registration = try JSONDecoder().decode(JSON.self, from: Data(contentsOf: recoveryURL))
            guard registration["scenario"].string == "WEB-REM-PARITY", registration["run_id"].string.hasPrefix("reminders-parity_"), registration["process_id"].number == Double(app.processIdentifier) else { throw SourceFailure.conflict }
            let candidates = sheets()
            guard candidates.count == 1 else { throw SourceFailure.conflict }
            let sheet = candidates[0]
            let cancel = find("Cancel", role: kAXButtonRole, within: sheet)
            let standard = find("Standard", role: kAXPopUpButtonRole, within: sheet)
            var fields: [AXUIElement] = []
            for field in find("New List", role: kAXTextFieldRole, within: sheet) + find("", role: kAXTextFieldRole, within: sheet) {
                guard ["", "New List"].contains(value(field, kAXValueAttribute)), !fields.contains(where: { CFEqual($0, field) }) else { continue }
                fields.append(field)
            }
            accessibilityDiagnostics["recovery_cancel_buttons"] = Double(cancel.count)
            accessibilityDiagnostics["recovery_standard_controls"] = Double(standard.count)
            accessibilityDiagnostics["recovery_default_name_fields"] = Double(fields.count)
            guard cancel.count == 1, standard.count == 1, fields.count == 1 else { throw SourceFailure.conflict }
            guard AXUIElementPerformAction(cancel[0], kAXPressAction as CFString) == .success else { throw SourceFailure.unavailable }
            let deadline = ContinuousClock.now.advanced(by: .seconds(3))
            while sheets().contains(where: { CFEqual($0, sheet) }), ContinuousClock.now < deadline {
                try await Task.sleep(for: .milliseconds(100))
            }
            guard !sheets().contains(where: { CFEqual($0, sheet) }) else { throw SourceFailure.conflict }
            try FileManager.default.removeItem(at: recoveryURL)
            accessibilityDiagnostics["recovered_owned_new_list_sheet"] = 1
        }
        let introduction = find("Continue", role: kAXButtonRole)
        if introduction.count == 1, (searchDiagnostics["introduction_heading"] ?? 0) > 0 {
            guard AXUIElementPerformAction(introduction[0], kAXPressAction as CFString) == .success else { throw SourceFailure.unavailable }
            accessibilityDiagnostics["introduction_dismissed"] = 1
            try await Task.sleep(for: .seconds(1))
        }
        for sheet in sheets() {
            let next = find("Continue", role: kAXButtonRole, within: sheet)
            guard next.count == 1, (searchDiagnostics["sheet_word_Location"] ?? 0) > 0, (searchDiagnostics["sheet_word_Reminders"] ?? 0) > 0 else { continue }
            accessibilityDiagnostics["location_consent_sheet"] = 1
            let cancel = elements(sheet, kAXCancelButtonAttribute)
            let declineNames = ["Cancel", "Not Now", "Don’t Allow", "Don't Allow", "Skip", "Set Up Later"]
            let decline = cancel.filter { element in declineNames.contains { name in [kAXTitleAttribute, kAXValueAttribute, kAXDescriptionAttribute].contains { value(element, $0) == name } } }
            accessibilityDiagnostics["location_decline_buttons"] = Double(decline.count)
            var actions: CFArray?
            let read = AXUIElementCopyActionNames(sheet, &actions)
            let canCancel = read == .success && (actions as? [String] ?? []).contains(kAXCancelAction as String)
            accessibilityDiagnostics["location_cancel_action"] = canCancel ? 1 : 0
            if decline.count == 1 {
                accessibilityDiagnostics["location_cancel_result"] = Double(AXUIElementPerformAction(decline[0], kAXPressAction as CFString).rawValue)
            } else if canCancel {
                accessibilityDiagnostics["location_cancel_result"] = Double(AXUIElementPerformAction(sheet, kAXCancelAction as CFString).rawValue)
            }
            if decline.count == 1 || canCancel {
                let deadline = ContinuousClock.now.advanced(by: .seconds(3))
                while sheets().contains(where: { CFEqual($0, sheet) }), ContinuousClock.now < deadline {
                    try await Task.sleep(for: .milliseconds(100))
                }
            }
            let cancellations: [(CGKeyCode, CGEventFlags, String)] = [(53, [], "escape"), (47, .maskCommand, "command_period")]
            for (code, flags, name) in cancellations {
                guard sheets().contains(where: { CFEqual($0, sheet) }), NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier else { break }
                if let down = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: true), let up = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: false) {
                    down.flags = flags; up.flags = flags
                    down.post(tap: .cghidEventTap); up.post(tap: .cghidEventTap)
                    accessibilityDiagnostics["location_" + name + "_sent"] = 1
                    let deadline = ContinuousClock.now.advanced(by: .seconds(3))
                    while sheets().contains(where: { CFEqual($0, sheet) }), ContinuousClock.now < deadline {
                        try await Task.sleep(for: .milliseconds(100))
                    }
                }
            }
            if sheets().contains(where: { CFEqual($0, sheet) }) { accessibilityDiagnostics["location_consent_blocked"] = 1 }
        }
        var minimizedCount = 0
        for window in elements(root, kAXWindowsAttribute) {
            var minimized: CFTypeRef?
            if AXUIElementCopyAttributeValue(window, kAXMinimizedAttribute as CFString, &minimized) == .success, minimized as? Bool == true {
                minimizedCount += 1
                guard AXUIElementSetAttributeValue(window, kAXMinimizedAttribute as CFString, kCFBooleanFalse) == .success else { throw SourceFailure.unavailable }
            }
            _ = AXUIElementPerformAction(window, kAXRaiseAction as CFString)
        }
        accessibilityDiagnostics["minimized_windows"] = Double(minimizedCount)
        accessibilityDiagnostics["sheet_count"] = Double(sheets().count)
        let sidebar = find("Show Sidebar", role: kAXMenuItemRole)
        if sidebar.count == 1 {
            accessibilityDiagnostics["show_sidebar_result"] = Double(AXUIElementPerformAction(sidebar[0], kAXPressAction as CFString).rawValue)
        }
        try await Task.sleep(for: .milliseconds(500))
    }

    func value(_ element: AXUIElement, _ key: String) -> String {
        var result: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, key as CFString, &result) == .success else { return "" }
        return (result as? NSAttributedString)?.string ?? result as? String ?? ""
    }

    func elements(_ element: AXUIElement, _ attribute: String) -> [AXUIElement] {
        var result: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, attribute as CFString, &result) == .success else { return [] }
        if let values = result as? [AXUIElement] { return values }
        if let result, CFGetTypeID(result) == AXUIElementGetTypeID() { return [unsafeDowncast(result, to: AXUIElement.self)] }
        return []
    }

    func find(_ name: String, role: String? = nil, within: AXUIElement? = nil) -> [AXUIElement] {
        var budget = 3000
        var visited: [AXUIElement] = []
        var partialLabels = 0
        var rows = 0
        var roles: [String: Double] = [:]
        let deadline = ContinuousClock.now.advanced(by: .seconds(3))
        func walk(_ element: AXUIElement, inSheet: Bool = false) -> [AXUIElement] {
            guard budget > 0, ContinuousClock.now < deadline, !visited.contains(where: { CFEqual($0, element) }) else { return [] }
            visited.append(element)
            budget -= 1
            let actualRole = value(element, kAXRoleAttribute)
            if actualRole == kAXRowRole { rows += 1 }
            roles["role_" + actualRole, default: 0] += 1
            let subrole = value(element, kAXSubroleAttribute)
            if !subrole.isEmpty { roles["subrole_" + subrole, default: 0] += 1 }
            let labels = [kAXTitleAttribute, kAXValueAttribute, kAXDescriptionAttribute].map { value(element, $0) }
            let sheet = inSheet || actualRole == kAXSheetRole
            if sheet {
                for word in ["iCloud", "Apple Intelligence", "Reminders", "Notifications", "Location", "Upgrade", "Permission", "Account", "Welcome", "What’s New", "Siri", "Suggested", "Suggestions", "Templates", "Shared", "Share", "Groceries", "Devices", "Default", "Continue", "Allow", "Cancel", "Not Now", "Enable", "Turn On"] where labels.contains(where: { $0.localizedCaseInsensitiveContains(word) }) {
                    roles["sheet_word_" + word, default: 0] += 1
                }
            }
            if actualRole == kAXStaticTextRole {
                let headings = ["welcome to reminders", "what’s new in reminders", "what's new in reminders", "what’s new", "what's new"]
                if labels.contains(where: { headings.contains($0.split(whereSeparator: { $0.isWhitespace }).joined(separator: " ").lowercased()) }) { roles["introduction_heading", default: 0] += 1 }
            }
            for control in ["Reminders", "Templates", "Search", "New List", "Welcome to Reminders", "Get Started", "Continue", "Upgrade", "Sign In", "iCloud", "Done", "Cancel", "Today", "All", "Scheduled", "Flagged", "No Reminders"] where labels.contains(control) {
                roles["control_" + control, default: 0] += 1
            }
            if subrole == kAXSearchFieldSubrole { roles["nonempty_search_fields", default: 0] += value(element, kAXValueAttribute).isEmpty ? 0 : 1 }
            if labels.contains(where: { $0.contains(name) }) { partialLabels += 1 }
            let matches = Self.matches(labels, name: name, title: title)
            if (role == nil || role == actualRole), matches, ![kAXApplicationRole, kAXWindowRole, kAXScrollAreaRole, kAXSplitGroupRole].contains(actualRole) { return [element] }
            let children = [kAXChildrenAttribute, kAXContentsAttribute, kAXRowsAttribute, kAXVisibleChildrenAttribute].flatMap { elements(element, $0) }
            return children.flatMap { walk($0, inSheet: sheet) }
        }
        let windows = elements(root, kAXWindowsAttribute)
        let menus = elements(root, kAXMenuBarAttribute)
        let roots = within.map { [$0] } ?? (role == kAXMenuBarItemRole || role == kAXMenuItemRole ? menus + windows : windows + menus)
        let matches = (roots.isEmpty ? [root] : roots).flatMap { walk($0) }
        searchDiagnostics = ["visited": Double(visited.count), "rows": Double(rows), "partial_labels": Double(partialLabels), "truncated": budget == 0 || ContinuousClock.now >= deadline ? 1 : 0]
        searchDiagnostics.merge(roles) { _, new in new }
        return matches
    }

    func sheets() -> [AXUIElement] {
        let candidates = elements(root, kAXWindowsAttribute).flatMap { elements($0, kAXChildrenAttribute) }.filter { value($0, kAXRoleAttribute) == kAXSheetRole }
        var focusedSheets: [AXUIElement] = []
        if var current = elements(root, kAXFocusedUIElementAttribute).first {
            for _ in 0..<20 {
                if value(current, kAXRoleAttribute) == kAXSheetRole { focusedSheets.append(current); break }
                guard let parent = elements(current, kAXParentAttribute).first else { break }
                current = parent
            }
        }
        var active = focusedSheets
        for candidate in candidates {
            var modal: CFTypeRef?
            if AXUIElementCopyAttributeValue(candidate, kAXModalAttribute as CFString, &modal) == .success, modal as? Bool == true,
               !active.contains(where: { CFEqual($0, candidate) }) { active.append(candidate) }
        }
        selectionDiagnostics["active_sheets"] = Double(active.count)
        selectionDiagnostics["sheet_candidates"] = Double(candidates.count)
        return active
    }

    func fence() throws {
        try gestureFence()
        var rows = 0
        for match in find(title) {
            var current = match
            for _ in 0..<12 {
                if value(current, kAXRoleAttribute) == kAXRowRole {
                    rows += 1
                    var selected: CFTypeRef?
                    if AXUIElementCopyAttributeValue(current, kAXSelectedAttribute as CFString, &selected) == .success, (selected as? Bool) == true {
                        selectionDiagnostics["owned_list_selected"] = 1
                        return
                    }
                    break
                }
                var parent: CFTypeRef?
                guard AXUIElementCopyAttributeValue(current, kAXParentAttribute as CFString, &parent) == .success, let parent, CFGetTypeID(parent) == AXUIElementGetTypeID() else { break }
                current = unsafeDowncast(parent, to: AXUIElement.self)
            }
        }
        selectionDiagnostics["owned_list_row_matches"] = Double(rows)
        selectionDiagnostics["owned_list_selected"] = 0
        throw SourceFailure.conflict
    }

    func activate() async throws {
        _ = app.activate()
        let deadline = ContinuousClock.now.advanced(by: .seconds(3))
        while NSWorkspace.shared.frontmostApplication?.processIdentifier != app.processIdentifier, ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(100))
        }
        try fence()
    }

    func gestureFence() throws {
        let activeSheets = sheets()
        guard activeSheets.allSatisfy({ sheet in ownedSheet.map { CFEqual($0, sheet) } == true }) else { throw SourceFailure.conflict }
        let foreground = NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier
        selectionDiagnostics["foreground"] = foreground ? 1 : 0
        guard foreground else { throw SourceFailure.conflict }
    }

    func reminderRow(_ name: String) throws -> AXUIElement {
        guard name.hasPrefix(title + " ") else { throw SourceFailure.invalid }
        let direct = find(name, role: kAXRowRole)
        var rows = direct
        if rows.isEmpty {
            for match in find(name) {
                var current = match
                for _ in 0..<12 {
                    if value(current, kAXRoleAttribute) == kAXRowRole {
                        if !rows.contains(where: { CFEqual($0, current) }) { rows.append(current) }
                        break
                    }
                    let parents = elements(current, kAXParentAttribute)
                    guard parents.count == 1 else { break }
                    current = parents[0]
                }
            }
        }
        selectionDiagnostics["row_matches"] = Double(rows.count)
        guard rows.count == 1 else { throw SourceFailure.conflict }
        return rows[0]
    }

    @discardableResult
    func selectReminder(_ name: String) async throws -> AXUIElement {
        try fence()
        let row = try reminderRow(name)
        let result = AXUIElementSetAttributeValue(row, kAXSelectedAttribute as CFString, kCFBooleanTrue)
        selectionDiagnostics["select_result"] = Double(result.rawValue)
        if result != .success {
            guard AXUIElementPerformAction(row, kAXPressAction as CFString) == .success else { throw SourceFailure.unsupported }
        }
        try await Task.sleep(for: .milliseconds(200))
        let selectedRow = try reminderRow(name)
        var selected: CFTypeRef?
        let read = AXUIElementCopyAttributeValue(selectedRow, kAXSelectedAttribute as CFString, &selected)
        selectionDiagnostics["selected"] = read == .success && selected as? Bool == true ? 1 : 0
        guard read == .success, selected as? Bool == true else { throw SourceFailure.conflict }
        return selectedRow
    }

    func drag(_ from: String, to: String) async throws {
        guard from.hasPrefix(title), to.hasPrefix(title) else { throw SourceFailure.invalid }
        _ = try await selectReminder(from)
        func point(_ name: String, before: Bool) throws -> CGPoint {
            let row = try reminderRow(name)
            let fields = find(name, role: kAXTextFieldRole, within: row) + find(name, role: kAXTextAreaRole, within: row)
            let target = !before && fields.count == 1 ? fields[0] : row
            var position: CFTypeRef?, size: CFTypeRef?
            guard AXUIElementCopyAttributeValue(target, kAXPositionAttribute as CFString, &position) == .success,
                  AXUIElementCopyAttributeValue(target, kAXSizeAttribute as CFString, &size) == .success,
                  let position, let size, CFGetTypeID(position) == AXValueGetTypeID(), CFGetTypeID(size) == AXValueGetTypeID() else { throw SourceFailure.conflict }
            var origin = CGPoint.zero, dimensions = CGSize.zero
            guard AXValueGetValue(unsafeDowncast(position, to: AXValue.self), .cgPoint, &origin), AXValueGetValue(unsafeDowncast(size, to: AXValue.self), .cgSize, &dimensions), dimensions.width > 0, dimensions.height > 0 else { throw SourceFailure.conflict }
            return CGPoint(x: origin.x + min(60, dimensions.width / 2), y: origin.y + (before ? 2 : dimensions.height / 2))
        }
        let start = try point(from, before: false), end = try point(to, before: true)
        try await activate()
        try fence()
        defer { CGEvent(mouseEventSource: nil, mouseType: .leftMouseUp, mouseCursorPosition: end, mouseButton: .left)?.post(tap: .cghidEventTap) }
        CGEvent(mouseEventSource: nil, mouseType: .leftMouseDown, mouseCursorPosition: start, mouseButton: .left)?.post(tap: .cghidEventTap)
        for index in 1...20 {
            try gestureFence()
            let fraction = Double(index) / 20
            let location = CGPoint(x: start.x + (end.x - start.x) * fraction, y: start.y + (end.y - start.y) * fraction)
            CGEvent(mouseEventSource: nil, mouseType: .leftMouseDragged, mouseCursorPosition: location, mouseButton: .left)?.post(tap: .cghidEventTap)
            try await Task.sleep(for: .milliseconds(30))
        }
        CGEvent(mouseEventSource: nil, mouseType: .leftMouseUp, mouseCursorPosition: end, mouseButton: .left)?.post(tap: .cghidEventTap)
        try await Task.sleep(for: .milliseconds(500))
    }

    func press(_ name: String, role: String) throws {
        try fence()
        let matches = find(name, role: role)
        guard matches.count == 1, AXUIElementPerformAction(matches[0], kAXPressAction as CFString) == .success else { throw SourceFailure.unsupported }
    }

    func setFocused(text: String) async throws {
        try fence()
        guard let sheet = ownedSheet else { throw SourceFailure.conflict }
        let deadline = ContinuousClock.now.advanced(by: .seconds(5))
        repeat {
            let fields = elements(root, kAXFocusedUIElementAttribute) + find("New List", role: kAXTextFieldRole, within: sheet)
            for field in fields {
                let role = value(field, kAXRoleAttribute), textValue = value(field, kAXValueAttribute)
                selectionDiagnostics["name_focus_" + role] = 1
                selectionDiagnostics["name_default"] = textValue.isEmpty || ["New List", "Untitled"].contains(textValue) ? 1 : 0
                if Self.editableRole(role), textValue.isEmpty || ["New List", "Untitled"].contains(textValue) {
                    try set(field, text: text)
                    return
                }
            }
            try await Task.sleep(for: .milliseconds(100))
        } while ContinuousClock.now < deadline
        throw SourceFailure.conflict
    }

    func pressAny(_ names: [String], role: String) throws {
        let available = names.filter { find($0, role: role).count == 1 }
        guard available.count == 1 else { throw SourceFailure.conflict }
        try press(available[0], role: role)
    }

    func set(_ element: AXUIElement, text: String) throws {
        try fence()
        guard AXUIElementSetAttributeValue(element, kAXFocusedAttribute as CFString, kCFBooleanTrue) == .success,
              AXUIElementSetAttributeValue(element, kAXValueAttribute as CFString, text as CFString) == .success else { throw SourceFailure.unsupported }
    }

    func key(_ code: CGKeyCode, flags: CGEventFlags = []) throws {
        try fence()
        guard let down = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: true), let up = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: false) else { throw SourceFailure.unavailable }
        down.flags = flags; up.flags = flags
        down.post(tap: .cghidEventTap); up.post(tap: .cghidEventTap)
    }

    func menu(_ menu: String, _ items: [String]) async throws {
        try press(menu, role: kAXMenuBarItemRole)
        try await Task.sleep(for: .milliseconds(100))
        let available = items.filter { find($0, role: kAXMenuItemRole).count == 1 }
        guard available.count == 1 else { throw SourceFailure.unsupported }
        try press(available[0], role: kAXMenuItemRole)
        if ["New List", "New List…", "New List..."].contains(available[0]) {
            pendingNewListSheet = true
            try await adoptNewListSheet()
            return
        }
        try await Task.sleep(for: .milliseconds(300))
        let activeSheets = sheets()
        if activeSheets.count == 1 { ownedSheet = activeSheets[0] }
    }

    func adoptNewListSheet() async throws {
        guard pendingNewListSheet else { throw SourceFailure.conflict }
        let deadline = ContinuousClock.now.advanced(by: .seconds(5))
        repeat {
            let active = sheets()
            if active.count == 1, find("Standard", role: kAXPopUpButtonRole, within: active[0]).count == 1, find("Cancel", role: kAXButtonRole, within: active[0]).count == 1 {
                ownedSheet = active[0]
                pendingNewListSheet = false
                return
            }
            try await Task.sleep(for: .milliseconds(100))
        } while ContinuousClock.now < deadline
        throw SourceFailure.conflict
    }
}
#endif
