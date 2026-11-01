#if LAB
import Foundation
import AppKit
import ApplicationServices
import CoreLocation
import CryptoKit
import EventKit
import CalendarSource
import SourceSupport

@MainActor
enum CalendarParityProof {
    static func run() async throws -> JSON {
        var checks: [String: JSON] = [:], reasons: [String: JSON] = [:], controls: [String: JSON] = [:]
        checks["accessibility_granted"] = .bool(AXIsProcessTrusted())
        checks["calendar_granted"] = .bool(EKEventStore.authorizationStatus(for: .event) == .fullAccess)
        guard checks.values.allSatisfy(\.bool) else {
            reasons["permissions"] = .string("Calendar full access and Accessibility must already be enabled for the signed Helper. This check never requests access.")
            return report(checks, reasons, controls)
        }
        let store = EKEventStore()
        let (accountIdentities, participantIdentities) = ownIdentities(store: store)
        let identities = Set(accountIdentities.values.flatMap { $0 })
        let title = "[ab-test] " + UUID().uuidString
        let calendar = EKCalendar(for: .event, eventStore: store)
        let defaultSource = store.defaultCalendarForNewEvents?.source
        let invitationSource = store.sources.first { source in
            [.calDAV, .exchange].contains(source.sourceType) && !(accountIdentities[source.sourceIdentifier] ?? []).isEmpty
        }
        guard let source = defaultSource.flatMap({ (accountIdentities[$0.sourceIdentifier] ?? []).isEmpty ? nil : $0 }) ?? invitationSource ?? defaultSource else { throw SourceFailure.unavailable }
        let sourceIdentities = accountIdentities[source.sourceIdentifier] ?? []
        calendar.title = title; calendar.source = source
        try store.saveCalendar(calendar, commit: true)
        let calendarID = calendar.calendarIdentifier
        defer {
            store.reset()
            if let owned = store.calendar(withIdentifier: calendarID), owned.title == title { try? store.removeCalendar(owned, commit: true) }
        }
        let event = EKEvent(eventStore: store)
        event.calendar = calendar; event.title = title + " event"
        event.startDate = Date().addingTimeInterval(86400); event.endDate = event.startDate.addingTimeInterval(3600)
        let eventStart = event.startDate!, eventEnd = event.endDate!
        let locationTitle = "[ab-test] location"
        let location = EKStructuredLocation(title: locationTitle)
        location.geoLocation = CLLocation(latitude: 41.88, longitude: -87.63)
        location.radius = 100
        event.structuredLocation = location
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("ab-test-calendar-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        defer { try? FileManager.default.removeItem(at: root) }
        var stage = "fixture"
        var fixtureUI: CalendarFixtureUI?
        do {
            try store.save(event, span: .thisEvent, commit: true)
            let identifier = event.calendarItemIdentifier, external = event.calendarItemExternalIdentifier ?? ""
            let eventTitle = event.title ?? ""
            let database = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Group Containers/group.com.apple.calendar/Calendar.sqlitedb")
            func row() throws -> [String: JSON]? {
                try CalendarStoreSnapshot.read(path: database.path).event(identifier: identifier, external: external, calendar: calendarID, detached: false)
            }
            stage = "show_owned_event"
            var saved: EKEvent?
            for _ in 0..<30 {
                store.reset()
                saved = store.calendarItem(withIdentifier: identifier) as? EKEvent
                if saved?.structuredLocation?.title == locationTitle,
                   saved?.structuredLocation?.geoLocation?.coordinate.latitude == 41.88,
                   saved?.structuredLocation?.geoLocation?.coordinate.longitude == -87.63,
                   saved?.structuredLocation?.radius == 100 { break }
                try await Task.sleep(for: .milliseconds(100))
            }
            controls["saved_event_found"] = .bool(saved != nil)
            controls["location_title_matches"] = .bool(saved?.structuredLocation?.title == locationTitle)
            controls["location_latitude_matches"] = .bool(saved?.structuredLocation?.geoLocation?.coordinate.latitude == 41.88)
            controls["location_longitude_matches"] = .bool(saved?.structuredLocation?.geoLocation?.coordinate.longitude == -87.63)
            controls["location_radius_matches"] = .bool(saved?.structuredLocation?.radius == 100)
            controls["location_radius_default"] = .bool(saved?.structuredLocation?.radius == 0)
            checks["structured_location_readback"] = .bool(saved?.structuredLocation?.title == locationTitle && saved?.structuredLocation?.geoLocation?.coordinate.latitude == 41.88 && saved?.structuredLocation?.geoLocation?.coordinate.longitude == -87.63 && saved?.structuredLocation?.radius == 100)
            controls["own_account_identity_count"] = .number(Double(identities.count))
            controls["own_source_identity_count"] = .number(Double(sourceIdentities.count))
            guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "com.apple.iCal") else { throw SourceFailure.unavailable }
            let configuration = NSWorkspace.OpenConfiguration()
            configuration.activates = true
            let app = try await NSWorkspace.shared.openApplication(at: url, configuration: configuration)
            try await Task.sleep(for: .seconds(1))
            let ui = CalendarFixtureUI(app: app, title: eventTitle, calendarTitle: title)
            fixtureUI = ui
            try await ui.openOwnedEvent()
            checks["owned_event_inspector"] = .bool(ui.hasOwnedInspector())
            try ui.fence()
            for name in ["travel time", "travel time:", "Travel Time", "Add Attachment…", "Add notes, URL or attachments", "Remove Attachment", "Accept", "Maybe", "Decline", "Add Invitees", "Add Invitees…", "Add Alert, Repeat, or Travel Time", "None", "Done", "Send"] {
                controls[name] = .bool(!ui.find(name).isEmpty)
            }
            checks["invitation_response_readback"] = .bool(false)
            reasons["invitation"] = .string("This run has no owned incoming invitation. EventKit creates ordinary events and exposes attendees and response status as read-only; existing invitations were left untouched.")
            do {
                stage = "travel_time"
                try await ui.revealTravel()
                try ui.travelPopup()
                controls["travel_menu_opened"] = .bool(true)
                try ui.pressAny(["15 minutes", "15 Minutes"], role: kAXMenuItemRole)
                controls["travel_duration_selected"] = .bool(true)
                try ui.key(48)
                try await ui.commit()
                controls["travel_edit_committed"] = .bool(true)
                try await ui.openOwnedEvent()
                try await ui.revealTravel()
                try ui.fence()
                checks["travel_time_readback"] = .bool(["15 minutes", "15 Minutes"].contains { ui.find($0, role: kAXPopUpButtonRole).count == 1 })
                try await ui.commit()
            } catch {
                reasons["travel_time"] = .string("Calendar could not change travel time on the test event. Keep Calendar open and retry this check.")
            }
            do {
                stage = "attachment_add"
                try await ui.openOwnedEvent()
                let filename = "ab-test-" + UUID().uuidString + ".txt"
                let file = root.appendingPathComponent(filename)
                let bytes = Data("[ab-test] calendar attachment".utf8)
                try bytes.write(to: file)
                let digest = SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
                if ui.find("Add Attachment…", role: kAXButtonRole).isEmpty {
                    try await ui.menu("Edit", ["Attach File…", "Add Attachment…", "Attach File..."])
                } else { try ui.press("Add Attachment…", role: kAXButtonRole) }
                controls["attachment_dialog_opened"] = .bool(true)
                try await Task.sleep(for: .milliseconds(200))
                try ui.post(5, flags: [.maskCommand, .maskShift])
                try await Task.sleep(for: .milliseconds(200))
                try ui.setFocused(file.path, requireInspector: false)
                try ui.post(36, flags: [])
                try await Task.sleep(for: .milliseconds(200))
                try ui.dialogPress("Open")
                controls["attachment_file_selected"] = .bool(true)
                try await ui.commit()
                let reader = CalendarStoreEnrichment()
                let observation: [String: JSON] = ["event": .object(["collection": .string("calendar.event"), "native_key": .string(identifier), "data": .object(["calendar_id": .object(["$ref": .object(["native_key": .string(calendarID)])])])])]
                let lookups: [String: JSON] = [identifier: .object(["external": .string(external)])]
                var attached = false
                for _ in 0..<60 {
                    let captured = await reader.enrich(observation, lookups: lookups, force: true)
                    attached = captured["event"]?["data"]["attachments"].array.contains { $0["filename"].string == filename && $0["blob"]["digest"].string == digest } == true
                    if attached { break }
                    try await Task.sleep(for: .milliseconds(100))
                }
                checks["attachment_add_readback"] = .bool(attached)
                stage = "attachment_remove"
                try await ui.openOwnedEvent()
                try ui.contextMenu(filename)
                try ui.pressAny(["Remove Attachment", "Delete Attachment"], role: kAXMenuItemRole)
                try await ui.commit()
                var removed = false
                for _ in 0..<60 {
                    let captured = await reader.enrich(observation, lookups: lookups, force: true)
                    let current = try row()
                    removed = captured["event"]?["data"]["attachments"] == .array([]) && current != nil
                    if removed { break }
                    try await Task.sleep(for: .milliseconds(100))
                }
                checks["attachment_remove_readback"] = .bool(removed)
            } catch {
                reasons["attachments"] = .string("Calendar could not change attachments on the test event. Keep Calendar open and retry this check.")
                if controls["attachment_dialog_opened"]?.bool == true, ui.find("Cancel", role: kAXButtonRole).count == 1 { try? ui.dialogPress("Cancel") }
            }
            stage = "invitation"
            if let attendee = participantIdentities[source.sourceIdentifier]?.sorted().first ?? sourceIdentities.sorted().first {
                let organizer = identities.sorted().first(where: { $0 != attendee }) ?? attendee
                try await ui.openOwnedEvent()
                let invitationTitle = title + " invitation"
                let invitationUI = CalendarFixtureUI(app: app, title: invitationTitle, calendarTitle: title)
                let date = DateFormatter()
                date.locale = Locale(identifier: "en_US_POSIX"); date.timeZone = TimeZone(secondsFromGMT: 0); date.dateFormat = "yyyyMMdd'T'HHmmss'Z'"
                let lines = ["BEGIN:VCALENDAR", "VERSION:2.0", "PRODID:-//AppleBridge//Calendar parity//EN", "METHOD:REQUEST", "BEGIN:VEVENT", "UID:" + UUID().uuidString, "DTSTAMP:" + date.string(from: Date()), "DTSTART:" + date.string(from: eventStart), "DTEND:" + date.string(from: eventEnd), "SUMMARY:" + invitationTitle, "ORGANIZER:mailto:" + organizer, "ATTENDEE;RSVP=TRUE;PARTSTAT=NEEDS-ACTION:mailto:" + attendee, "END:VEVENT", "END:VCALENDAR", ""]
                let invitationFile = root.appendingPathComponent("[ab-test]-invitation.ics")
                try Data(lines.joined(separator: "\r\n").utf8).write(to: invitationFile)
                stage = "invitation_import"
                try await ui.importInvitation(invitationFile, calendarTitle: title)
                controls["invitation_import_completed"] = .bool(true)
                stage = "invitation_participants"
                store.reset()
                guard let importedCalendar = store.calendar(withIdentifier: calendarID) else { throw SourceFailure.unavailable }
                let query = store.predicateForEvents(withStart: eventStart.addingTimeInterval(-1), end: eventEnd.addingTimeInterval(1), calendars: [importedCalendar])
                let invitations = store.events(matching: query).filter { $0.title == invitationTitle }
                controls["owned_invitation_count"] = .number(Double(invitations.count))
                controls["invitation_attendee_count"] = .number(Double(invitations.first?.attendees?.count ?? 0))
                controls["invitation_current_user_count"] = .number(Double(invitations.first?.attendees?.filter(\.isCurrentUser).count ?? 0))
                guard invitations.count == 1, let invitation = invitations.first,
                      let participants = invitation.attendees, participants.count == 1,
                      participants[0].isCurrentUser,
                      participants[0].url.absoluteString.lowercased() == "mailto:" + attendee else { throw SourceFailure.conflict }
                checks["invitation_self_attendee_only"] = .bool(true)
                let invitationID = invitation.calendarItemIdentifier
                let before = participants[0].participantStatus
                controls["invitation_already_accepted"] = .bool(before == .accepted)
                stage = "invitation_response"
                try await invitationUI.openOwnedEvent()
                for name in ["Accept", "Maybe", "Decline", "Reply", "OK"] { controls[name] = .bool(!invitationUI.find(name).isEmpty) }
                try invitationUI.press("Accept", role: kAXButtonRole)
                for _ in 0..<60 {
                    store.reset()
                    if let current = store.calendarItem(withIdentifier: invitationID) as? EKEvent,
                       current.title == invitationTitle, current.calendar.calendarIdentifier == calendarID,
                       let attendees = current.attendees, attendees.count == 1,
                       attendees[0].isCurrentUser, attendees[0].participantStatus == .accepted, before != .accepted {
                        checks["invitation_response_readback"] = .bool(true)
                        reasons.removeValue(forKey: "invitation")
                        break
                    }
                    try await Task.sleep(for: .milliseconds(200))
                }
            } else {
                reasons["invitation"] = .string("EventKit could not identify Johan's own account attendee address. No invitation was created or sent.")
            }
        } catch {
            reasons[stage] = .string("Calendar could not complete the test event action. Keep Calendar open and retry this check.")
            if let app = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.iCal").first {
                let ui = CalendarFixtureUI(app: app, title: title + " event")
                for name in [title + " event", "travel time", "travel time:", "Travel Time", "Add Alert, Repeat, or Travel Time", "Add Attachment…", "Add notes, URL or attachments", "Accept", "Maybe", "Decline", "None", "Done", "Send", "Open", "Cancel", "Import", "OK"] {
                    controls[name == title + " event" ? "owned_title_visible" : name] = .number(Double(ui.find(name).count))
                }
                controls["popup_count"] = .number(Double(ui.findRole(kAXPopUpButtonRole).count))
            }
        }
        if let app = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.iCal").first {
            let ui = CalendarFixtureUI(app: app, title: title + " event")
            if ui.find("Cancel", role: kAXButtonRole).count == 1 { try? ui.dialogPress("Cancel") }
        }
        controls.merge(fixtureUI?.diagnostics ?? [:]) { _, latest in latest }
        do {
            store.reset()
            let query = store.predicateForEvents(withStart: eventStart.addingTimeInterval(-1), end: eventEnd.addingTimeInterval(1), calendars: nil)
            let ownedTitles = [title + " event", title + " invitation"]
            for ownedEvent in store.events(matching: query) where ownedTitles.contains(ownedEvent.title ?? "") {
                try store.remove(ownedEvent, span: .thisEvent, commit: true)
            }
            if let owned = store.calendar(withIdentifier: calendarID), owned.title == title { try store.removeCalendar(owned, commit: true) }
            store.reset()
            checks["owned_calendar_cleanup"] = .bool(store.calendar(withIdentifier: calendarID) == nil)
            checks["owned_events_cleanup"] = .bool(!store.events(matching: query).contains { ownedTitles.contains($0.title ?? "") })
        } catch {
            checks["owned_calendar_cleanup"] = .bool(false)
            reasons["cleanup"] = .string("Calendar could not remove this run's test calendar. Retry cleanup before running another check.")
        }
        return report(checks, reasons, controls)
    }

    private static func ownIdentities(store: EKEventStore) -> ([String: Set<String>], [String: Set<String>]) {
        var identities: [String: Set<String>] = [:], participantIdentities: [String: Set<String>] = [:]
        let path = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Group Containers/group.com.apple.calendar/Calendar.sqlitedb").path
        let owners = (try? CalendarStoreSnapshot.ownerAddresses(path: path)) ?? [:]
        for calendar in store.calendars(for: .event) where !calendar.source.isDelegate && [.calDAV, .exchange].contains(calendar.source.sourceType) {
            let key = UUID(uuidString: calendar.calendarIdentifier)?.uuidString ?? calendar.calendarIdentifier
            identities[calendar.source.sourceIdentifier, default: []].formUnion(owners[key] ?? [])
        }
        for interval in CalendarEventKitFields.identitySearchIntervals(around: Date()) {
            let predicate = store.predicateForEvents(withStart: interval.start, end: interval.end, calendars: nil)
            for event in store.events(matching: predicate) {
                for participant in (event.attendees ?? []) + [event.organizer].compactMap({ $0 }) where participant.isCurrentUser && participant.url.scheme?.lowercased() == "mailto" && !event.calendar.source.isDelegate {
                    let address = String(participant.url.absoluteString.dropFirst(7)).removingPercentEncoding?.lowercased() ?? ""
                    if !address.isEmpty, !address.contains("\n"), !address.contains("\r") {
                        identities[event.calendar.source.sourceIdentifier, default: []].insert(address)
                        participantIdentities[event.calendar.source.sourceIdentifier, default: []].insert(address)
                    }
                }
            }
        }
        return (identities, participantIdentities)
    }

    private static func report(_ checks: [String: JSON], _ reasons: [String: JSON], _ controls: [String: JSON]) -> JSON {
        let required = ["accessibility_granted", "calendar_granted", "structured_location_readback", "invitation_self_attendee_only", "invitation_response_readback", "travel_time_readback", "attachment_add_readback", "attachment_remove_readback", "owned_calendar_cleanup", "owned_events_cleanup"]
        return .object(["passed": .bool(required.allSatisfy { checks[$0]?.bool == true }), "checks": .object(checks), "reasons": .object(reasons), "controls": .object(controls)])
    }
}

@MainActor
final class CalendarFixtureUI {
    let app: NSRunningApplication
    let root: AXUIElement
    let title: String
    let calendarTitle: String?
    var diagnostics: [String: JSON] = [:]
    private var inspectorCache: AXUIElement?

    init(app: NSRunningApplication, title: String, calendarTitle: String? = nil) {
        self.app = app; self.title = title; self.calendarTitle = calendarTitle
        root = AXUIElementCreateApplication(app.processIdentifier)
        AXUIElementSetMessagingTimeout(root, 0.1)
    }

    func value(_ element: AXUIElement, _ key: String) -> String {
        var result: CFTypeRef?
        guard AXUIElementCopyAttributeValue(element, key as CFString, &result) == .success else { return "" }
        return (result as? NSAttributedString)?.string ?? result as? String ?? (result as? NSNumber)?.stringValue ?? ""
    }

    func find(_ name: String, role: String? = nil, containing: Bool = false, within: AXUIElement? = nil) -> [AXUIElement] {
        var budget = 3000
        let deadline = ContinuousClock.now.advanced(by: .seconds(2))
        func walk(_ element: AXUIElement) -> [AXUIElement] {
            guard budget > 0, ContinuousClock.now < deadline else { return [] }; budget -= 1
            if (role == nil || value(element, kAXRoleAttribute) == role), [kAXTitleAttribute, kAXValueAttribute, kAXDescriptionAttribute].contains(where: { containing ? value(element, $0).contains(name) : value(element, $0) == name }) { return [element] }
            var children: CFTypeRef?
            AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &children)
            return (children as? [AXUIElement] ?? []).flatMap(walk)
        }
        return walk(within ?? root)
    }

    func openOwnedEvent() async throws {
        app.activate()
        for _ in 0..<20 {
            if NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier { break }
            try await Task.sleep(for: .milliseconds(100))
        }
        diagnostics["calendar_frontmost"] = .bool(NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier)
        var windows: CFTypeRef?
        for _ in 0..<50 {
            AXUIElementCopyAttributeValue(root, kAXWindowsAttribute as CFString, &windows)
            if !(windows as? [AXUIElement] ?? []).isEmpty { break }
            try await Task.sleep(for: .milliseconds(100))
        }
        diagnostics["calendar_window_count"] = .number(Double((windows as? [AXUIElement])?.count ?? 0))
        if hasOwnedInspector() { return }
        if let calendarTitle {
            var calendars = find(calendarTitle, role: kAXCheckBoxRole)
            let toggles = find("Calendars", role: kAXButtonRole)
            if calendars.isEmpty, toggles.count == 1, value(toggles[0], kAXValueAttribute) == "0" {
                try dialogPress("Calendars")
                try await Task.sleep(for: .milliseconds(200))
                calendars = find(calendarTitle, role: kAXCheckBoxRole)
            }
            diagnostics["owned_calendar_checkbox_count"] = .number(Double(calendars.count))
            if calendars.count == 1, value(calendars[0], kAXValueAttribute) == "0" {
                guard AXUIElementPerformAction(calendars[0], kAXPressAction as CFString) == .success else { throw SourceFailure.unsupported }
                try await Task.sleep(for: .milliseconds(200))
            }
        }
        try post(3, flags: .maskCommand)
        var searches: [AXUIElement] = []
        for _ in 0..<20 {
            searches = findRole(kAXTextFieldRole).filter { value($0, kAXSubroleAttribute) == kAXSearchFieldSubrole }
            if searches.count == 1 { break }
            try await Task.sleep(for: .milliseconds(100))
        }
        diagnostics["calendar_search_field_count"] = .number(Double(searches.count))
        guard searches.count == 1 else { throw SourceFailure.conflict }
        _ = AXUIElementSetAttributeValue(searches[0], kAXFocusedAttribute as CFString, kCFBooleanTrue)
        try typeSearch(title.replacingOccurrences(of: "[ab-test] ", with: ""))
        diagnostics["calendar_search_entered"] = .bool(true)
        try await Task.sleep(for: .milliseconds(200))
        try post(36, flags: [])
        try await Task.sleep(for: .seconds(1))
        var results: [AXUIElement] = []
        for _ in 0..<40 {
            results = find(title, containing: true).filter { value($0, kAXSubroleAttribute) != kAXSearchFieldSubrole }
            if !results.isEmpty { break }
            try await Task.sleep(for: .milliseconds(250))
        }
        diagnostics["owned_search_result_count"] = .number(Double(results.count))
        for result in results {
            let key = "owned_search_result_" + value(result, kAXRoleAttribute)
            diagnostics[key] = .number((diagnostics[key]?.number ?? 0) + 1)
        }
        guard let result = results.first else { throw SourceFailure.unavailable }
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier else { throw SourceFailure.conflict }
        var candidate = result
        for _ in 0..<4 {
            if AXUIElementSetAttributeValue(candidate, kAXSelectedAttribute as CFString, kCFBooleanTrue) == .success {
                diagnostics["owned_result_selected"] = .bool(true)
            }
            if AXUIElementPerformAction(candidate, kAXShowMenuAction as CFString) == .success {
                try await Task.sleep(for: .milliseconds(200))
                let info = find("Get Info", role: kAXMenuItemRole)
                if info.count == 1, AXUIElementPerformAction(info[0], kAXPressAction as CFString) == .success {
                    diagnostics["owned_result_info_succeeded"] = .bool(true)
                    try await Task.sleep(for: .milliseconds(400))
                    if hasOwnedInspector() { try fence(); return }
                } else { try post(53, flags: []) }
            }
            if AXUIElementPerformAction(candidate, kAXPressAction as CFString) == .success {
                diagnostics["owned_result_press_succeeded"] = .bool(true)
                try await Task.sleep(for: .milliseconds(200))
                if !hasOwnedInspector() {
                    try post(34, flags: .maskCommand)
                    try await Task.sleep(for: .milliseconds(400))
                }
                if hasOwnedInspector() { try fence(); return }
            }
            var parent: CFTypeRef?
            guard AXUIElementCopyAttributeValue(candidate, kAXParentAttribute as CFString, &parent) == .success,
                  let parent, CFGetTypeID(parent) == AXUIElementGetTypeID() else { break }
            candidate = unsafeDowncast(parent, to: AXUIElement.self)
            if [kAXWindowRole, kAXApplicationRole, kAXTableRole, kAXOutlineRole, kAXScrollAreaRole].contains(value(candidate, kAXRoleAttribute)) { break }
        }
        var position: CFTypeRef?, size: CFTypeRef?
        guard AXUIElementCopyAttributeValue(result, kAXPositionAttribute as CFString, &position) == .success,
              AXUIElementCopyAttributeValue(result, kAXSizeAttribute as CFString, &size) == .success,
              let position, let size else { throw SourceFailure.unsupported }
        var point = CGPoint.zero, dimensions = CGSize.zero
        guard AXValueGetValue(unsafeDowncast(position, to: AXValue.self), .cgPoint, &point),
              AXValueGetValue(unsafeDowncast(size, to: AXValue.self), .cgSize, &dimensions) else { throw SourceFailure.unsupported }
        point.x += dimensions.width / 2; point.y += dimensions.height / 2
        for type in [CGEventType.leftMouseDown, .leftMouseUp] {
            guard let event = CGEvent(mouseEventSource: nil, mouseType: type, mouseCursorPosition: point, mouseButton: .left) else { throw SourceFailure.unavailable }
            event.setIntegerValueField(.mouseEventClickState, value: 2)
            event.post(tap: .cghidEventTap)
        }
        try await Task.sleep(for: .milliseconds(400))
        if !hasOwnedInspector() { try post(34, flags: .maskCommand); try await Task.sleep(for: .milliseconds(400)) }
        try fence()
    }

    func commit() async throws {
        try fence()
        if inspectorFind("Done", role: kAXButtonRole).count == 1 { try press("Done", role: kAXButtonRole) }
        else { try post(53, flags: []) }
        try await Task.sleep(for: .milliseconds(500))
    }

    func revealTravel() async throws {
        try fence()
        if !inspectorFind("travel time", role: kAXStaticTextRole).isEmpty || !inspectorFind("travel time:", role: kAXStaticTextRole).isEmpty { return }
        for name in ["Add Alert, Repeat, or Travel Time", "Add Alert, Repeat or Travel Time"] {
            let matches = inspectorFind(name)
            if matches.count == 1 {
                guard AXUIElementPerformAction(matches[0], kAXPressAction as CFString) == .success else { throw SourceFailure.unsupported }
                try await Task.sleep(for: .milliseconds(200))
                return
            }
        }
    }

    func importInvitation(_ file: URL, calendarTitle: String) async throws {
        try fence()
        try await menu("File", ["Import…", "Import..."])
        try await Task.sleep(for: .milliseconds(200))
        try post(5, flags: [.maskCommand, .maskShift])
        try await Task.sleep(for: .milliseconds(200))
        try setFocused(file.path, requireInspector: false)
        try post(36, flags: [])
        try await Task.sleep(for: .milliseconds(200))
        try dialogPress("Import")
        try await Task.sleep(for: .milliseconds(500))
        let popups = findRole(kAXPopUpButtonRole)
        guard popups.count == 1, AXUIElementPerformAction(popups[0], kAXPressAction as CFString) == .success else { throw SourceFailure.conflict }
        try await Task.sleep(for: .milliseconds(100))
        let calendars = find(calendarTitle, role: kAXMenuItemRole)
        guard calendars.count == 1, AXUIElementPerformAction(calendars[0], kAXPressAction as CFString) == .success else { throw SourceFailure.conflict }
        try dialogPress("OK")
        try await Task.sleep(for: .seconds(1))
    }

    func findRole(_ role: String) -> [AXUIElement] {
        var budget = 3000
        let deadline = ContinuousClock.now.advanced(by: .seconds(2))
        func walk(_ element: AXUIElement) -> [AXUIElement] {
            guard budget > 0, ContinuousClock.now < deadline else { return [] }; budget -= 1
            var children: CFTypeRef?
            AXUIElementCopyAttributeValue(element, kAXChildrenAttribute as CFString, &children)
            let descendants = (children as? [AXUIElement] ?? []).flatMap(walk)
            return (value(element, kAXRoleAttribute) == role ? [element] : []) + descendants
        }
        return walk(root)
    }

    func dialogPress(_ name: String) throws {
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier else { throw SourceFailure.conflict }
        let matches = find(name, role: kAXButtonRole)
        guard matches.count == 1, AXUIElementPerformAction(matches[0], kAXPressAction as CFString) == .success else { throw SourceFailure.unsupported }
    }

    func ownedInspector() -> AXUIElement? {
        if let cached = inspectorCache, !find(title, containing: true, within: cached).isEmpty { return cached }
        inspectorCache = nil
        let fields = find(title, containing: true).filter { value($0, kAXSubroleAttribute) != kAXSearchFieldSubrole }
        for field in fields {
            var current = field
            for _ in 0..<6 {
                let role = value(current, kAXRoleAttribute)
                if role == kAXApplicationRole { break }
                if role == kAXWindowRole, !value(current, kAXTitleAttribute).contains(title) { break }
                let anchors = ["Done", "Accept", "Maybe", "Decline", "Add Invitees", "Add Invitees…", "Add Attachment…", "Add Alert, Repeat, or Travel Time", "Add notes, URL or attachments"]
                if anchors.contains(where: { !find($0, within: current).isEmpty }) { inspectorCache = current; return current }
                var parent: CFTypeRef?
                guard AXUIElementCopyAttributeValue(current, kAXParentAttribute as CFString, &parent) == .success,
                      let parent, CFGetTypeID(parent) == AXUIElementGetTypeID() else { break }
                current = unsafeDowncast(parent, to: AXUIElement.self)
            }
        }
        return nil
    }

    func hasOwnedInspector() -> Bool { ownedInspector() != nil }

    func inspectorFind(_ name: String, role: String? = nil) -> [AXUIElement] {
        guard let inspector = ownedInspector() else { return [] }
        return find(name, role: role, within: inspector)
    }

    func fence() throws {
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier, hasOwnedInspector() else { throw SourceFailure.conflict }
    }

    func navigationInfo() throws {
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier, !find(title).isEmpty else { throw SourceFailure.conflict }
        try post(34, flags: .maskCommand)
    }

    func post(_ code: CGKeyCode, flags: CGEventFlags) throws {
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier else { throw SourceFailure.conflict }
        guard let down = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: true), let up = CGEvent(keyboardEventSource: nil, virtualKey: code, keyDown: false) else { throw SourceFailure.unavailable }
        down.flags = flags; up.flags = flags
        down.postToPid(app.processIdentifier); up.postToPid(app.processIdentifier)
    }

    func key(_ code: CGKeyCode, flags: CGEventFlags = []) throws { try fence(); try post(code, flags: flags) }

    func press(_ name: String, role: String) throws {
        try fence()
        let matches = role == kAXButtonRole ? inspectorFind(name, role: role) : find(name, role: role)
        guard matches.count == 1, AXUIElementPerformAction(matches[0], kAXPressAction as CFString) == .success else { throw SourceFailure.unsupported }
    }

    func pressAny(_ names: [String], role: String) throws {
        let available = names.filter { find($0, role: role).count == 1 }
        guard available.count == 1 else { throw SourceFailure.unsupported }
        try press(available[0], role: role)
    }

    func menu(_ name: String, _ items: [String]) async throws {
        try press(name, role: kAXMenuBarItemRole)
        try await Task.sleep(for: .milliseconds(100))
        try pressAny(items, role: kAXMenuItemRole)
    }

    func setFocused(_ text: String, requireInspector: Bool = true) throws {
        if requireInspector { try fence() }
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == app.processIdentifier else { throw SourceFailure.conflict }
        var focused: CFTypeRef?
        guard AXUIElementCopyAttributeValue(root, kAXFocusedUIElementAttribute as CFString, &focused) == .success, let focused, CFGetTypeID(focused) == AXUIElementGetTypeID() else { throw SourceFailure.conflict }
        let field = unsafeDowncast(focused, to: AXUIElement.self)
        guard value(field, kAXRoleAttribute) == kAXTextFieldRole, AXUIElementSetAttributeValue(field, kAXValueAttribute as CFString, text as CFString) == .success else { throw SourceFailure.unsupported }
    }

    func typeSearch(_ text: String) throws {
        var focused: CFTypeRef?
        guard AXUIElementCopyAttributeValue(root, kAXFocusedUIElementAttribute as CFString, &focused) == .success,
              let focused, CFGetTypeID(focused) == AXUIElementGetTypeID() else { throw SourceFailure.conflict }
        let field = unsafeDowncast(focused, to: AXUIElement.self)
        guard value(field, kAXRoleAttribute) == kAXTextFieldRole, value(field, kAXSubroleAttribute) == kAXSearchFieldSubrole else { throw SourceFailure.conflict }
        try post(0, flags: .maskCommand)
        let characters = Array(text.utf16)
        for down in [true, false] {
            guard let event = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: down) else { throw SourceFailure.unavailable }
            event.flags = []
            characters.withUnsafeBufferPointer { buffer in
                if let base = buffer.baseAddress { event.keyboardSetUnicodeString(stringLength: buffer.count, unicodeString: base) }
            }
            event.postToPid(app.processIdentifier)
        }
    }

    func travelPopup() throws {
        try fence()
        let labels = ["travel time", "travel time:", "Travel Time", "Travel Time:"].flatMap { inspectorFind($0, role: kAXStaticTextRole) }
        guard labels.count == 1 else { throw SourceFailure.unsupported }
        var current = labels[0]
        for _ in 0..<3 {
            var parent: CFTypeRef?
            guard AXUIElementCopyAttributeValue(current, kAXParentAttribute as CFString, &parent) == .success, let parent, CFGetTypeID(parent) == AXUIElementGetTypeID() else { throw SourceFailure.conflict }
            current = unsafeDowncast(parent, to: AXUIElement.self)
            var children: CFTypeRef?
            AXUIElementCopyAttributeValue(current, kAXChildrenAttribute as CFString, &children)
            let popups = (children as? [AXUIElement] ?? []).filter { value($0, kAXRoleAttribute) == kAXPopUpButtonRole }
            if popups.count == 1 {
                guard AXUIElementPerformAction(popups[0], kAXPressAction as CFString) == .success else { throw SourceFailure.unsupported }
                return
            }
        }
        throw SourceFailure.unsupported
    }

    func contextMenu(_ name: String) throws {
        try fence()
        let matches = inspectorFind(name)
        guard matches.count == 1, AXUIElementPerformAction(matches[0], kAXShowMenuAction as CFString) == .success else { throw SourceFailure.unsupported }
    }
}
#endif
