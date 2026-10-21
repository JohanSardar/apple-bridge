import Foundation
@preconcurrency import Contacts
import CryptoKit
import ContactsHistory
import SourceSupport

@globalActor
public actor ContactsActor {
    public static let shared = ContactsActor()
}

@ContactsActor
public final class ContactsSource: Source {
    public nonisolated let name = "contacts"
    private let store = CNContactStore()
    private let commandsEnabled: Bool
    private let trace: CaptureTrace
    private var generation = UUID().uuidString
    private var token: Data?
    private var committed: [String: JSON] = [:]
    private var blobs: [String: Data] = [:]
    private var scanning = false
    private let baselineURL: URL
    private static let operations = ["contacts.create", "contacts.update", "contacts.delete"]
    public nonisolated static let coverage: JSON = .object(["fields": .string("partial"), "missing": .array([.string("note: unavailable without restricted entitlement"), .string("writability: conservatively limited to default container")])])
    private static let scalarKeys = ["given_name": CNContactGivenNameKey, "middle_name": CNContactMiddleNameKey, "family_name": CNContactFamilyNameKey, "previous_family_name": CNContactPreviousFamilyNameKey, "nickname": CNContactNicknameKey, "name_prefix": CNContactNamePrefixKey, "name_suffix": CNContactNameSuffixKey, "phonetic_given_name": CNContactPhoneticGivenNameKey, "phonetic_middle_name": CNContactPhoneticMiddleNameKey, "phonetic_family_name": CNContactPhoneticFamilyNameKey, "phonetic_organization": CNContactPhoneticOrganizationNameKey, "organization": CNContactOrganizationNameKey, "job_title": CNContactJobTitleKey, "department": CNContactDepartmentNameKey]
    private static var keys: [any CNKeyDescriptor] {
        (Array(scalarKeys.values) + [CNContactIdentifierKey, CNContactTypeKey, CNContactPhoneNumbersKey, CNContactEmailAddressesKey, CNContactPostalAddressesKey, CNContactUrlAddressesKey, CNContactBirthdayKey, CNContactNonGregorianBirthdayKey, CNContactDatesKey, CNContactSocialProfilesKey, CNContactInstantMessageAddressesKey, CNContactRelationsKey, CNContactImageDataKey, CNContactThumbnailImageDataKey, CNContactImageDataAvailableKey]).map { $0 as NSString }
    }

    public nonisolated init(home: URL, commandsEnabled: Bool = false, trace: CaptureTrace = CaptureTrace()) throws {
        self.trace = trace
        self.commandsEnabled = commandsEnabled
        baselineURL = home.appendingPathComponent("contacts-baseline.json")
    }

    private var authorized: Bool { CNContactStore.authorizationStatus(for: .contacts) == .authorized }

    public func open() async -> SourceReadiness {
        SourceReadiness(state: authorized ? "ready" : "permission_missing", coverage: Self.coverage, capabilities: authorized && commandsEnabled ? Self.operations : [], reason: authorized ? "" : "contacts_full_access_required")
    }

    private func hash(_ bytes: Data) -> String { SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined() }
    private func hash(_ value: JSON) throws -> String {
        let encoder = JSONEncoder(); encoder.outputFormatting = [.sortedKeys]
        return hash(try encoder.encode(value))
    }
    private func date(_ value: DateComponents?) -> JSON {
        guard let value else { return .null }
        var fields: [String: JSON] = [:]
        if let year = value.year { fields["year"] = .number(Double(year)) }
        if let month = value.month { fields["month"] = .number(Double(month)) }
        if let day = value.day { fields["day"] = .number(Double(day)) }
        if let leap = value.isLeapMonth { fields["is_leap_month"] = .bool(leap) }
        if let calendar = value.calendar { fields["calendar"] = .string(String(describing: calendar.identifier)) }
        return .object(fields)
    }
    private func labeled<T>(_ values: [CNLabeledValue<T>], convert: (T) -> JSON) -> JSON {
        .array(values.map { .object(["label": $0.label.map(JSON.string) ?? .null, "value": convert($0.value)]) })
    }
    private func container(_ contact: CNContact) throws -> String {
        let values = try store.containers(matching: CNContainer.predicateForContainerOfContact(withIdentifier: contact.identifier))
        return values.count == 1 ? values[0].identifier : ""
    }
    private func data(_ contact: CNContact, containerID: String? = nil) throws -> JSON {
        guard contact.areKeysAvailable(Self.keys) else { throw SourceFailure.unavailable }
        var fields = Self.scalarKeys.mapValues { JSON.string(contact.value(forKey: $0) as? String ?? "") }
        fields["contact_type"] = .string(contact.contactType == .organization ? "organization" : "person")
        fields["note"] = .string("unavailable")
        fields["phones"] = labeled(contact.phoneNumbers) { .string($0.stringValue) }
        fields["emails"] = labeled(contact.emailAddresses) { .string($0 as String) }
        fields["urls"] = labeled(contact.urlAddresses) { .string($0 as String) }
        fields["postal_addresses"] = labeled(contact.postalAddresses) { .object(["street": .string($0.street), "sub_locality": .string($0.subLocality), "city": .string($0.city), "sub_administrative_area": .string($0.subAdministrativeArea), "state": .string($0.state), "postal_code": .string($0.postalCode), "country": .string($0.country), "iso_country_code": .string($0.isoCountryCode)]) }
        fields["birthday"] = date(contact.birthday)
        fields["non_gregorian_birthday"] = date(contact.nonGregorianBirthday)
        fields["dates"] = labeled(contact.dates) { date($0 as DateComponents) }
        fields["social_profiles"] = labeled(contact.socialProfiles) { .object(["url": .string($0.urlString), "username": .string($0.username), "user_identifier": .string($0.userIdentifier), "service": .string($0.service)]) }
        fields["instant_messages"] = labeled(contact.instantMessageAddresses) { .object(["username": .string($0.username), "service": .string($0.service)]) }
        fields["relations"] = labeled(contact.contactRelations) { .string($0.name) }
        let resolvedContainer: String
        if let containerID { resolvedContainer = containerID } else { resolvedContainer = try container(contact) }
        fields["container_id"] = .string(resolvedContainer)
        fields["writable"] = .bool(!resolvedContainer.isEmpty && resolvedContainer == store.defaultContainerIdentifier())
        fields["image"] = .null
        if let image = contact.imageData {
            let digest = hash(image)
            blobs["contacts-image:" + digest] = image
            fields["image"] = .object(["digest": .string(digest), "state": .string("present")])
        }
        if let image = contact.thumbnailImageData {
            let digest = hash(image)
            blobs["contacts-image:" + digest] = image
            fields["thumbnail_image"] = .object(["digest": .string(digest), "state": .string("present")])
        } else { fields["thumbnail_image"] = .null }
        return .object(fields)
    }
    private func observation(_ collection: String, _ key: String, _ value: JSON) -> JSON {
        .object(["kind": .string("upsert"), "collection": .string(collection), "native_key": .string(key), "scoped_alias": .string("contacts/" + collection + "/" + key), "data": value, "observed_at": .string(timestamp())])
    }
    private func contacts(_ predicate: NSPredicate? = nil, identifiersOnly: Bool = false) throws -> [CNContact] {
        guard authorized else { throw SourceFailure.permissionMissing }
        let request = CNContactFetchRequest(keysToFetch: identifiersOnly ? [CNContactIdentifierKey as NSString] : Self.keys)
        request.unifyResults = true; request.predicate = predicate
        var result: [CNContact] = []
        try store.enumerateContacts(with: request) { contact, _ in result.append(contact) }
        return result
    }
    private func history(_ starting: Data?) throws -> AppleBridgeContactsHistoryBatch {
        guard authorized else { throw SourceFailure.permissionMissing }
        let request = CNChangeHistoryFetchRequest()
        request.startingToken = starting; request.shouldUnifyResults = true; request.includeGroupChanges = true
        request.additionalContactKeyDescriptors = [CNContactIdentifierKey as NSString]
        guard let result = AppleBridgeContactsHistory.fetch(store: store, request: request) else { throw SourceFailure.unavailable }
        return result
    }
    private func fresh(_ key: String) throws -> CNContact {
        guard authorized else { throw SourceFailure.permissionMissing }
        return try store.unifiedContact(withIdentifier: key, keysToFetch: Self.keys)
    }
    private func commandContact(_ key: String) throws -> CNContact {
        do { return try fresh(key) }
        catch let error as NSError {
            if error.domain == CNErrorDomain && error.code == CNError.Code.recordDoesNotExist.rawValue { throw SourceFailure.conflict }
            throw error
        }
    }
    private func isAbsent(_ key: String) throws -> Bool {
        do { _ = try fresh(key); return false }
        catch let error as NSError {
            guard authorized else { throw SourceFailure.permissionMissing }
            guard error.domain == CNErrorDomain && error.code == CNError.Code.recordDoesNotExist.rawValue else { throw error }
            return true
        }
    }
    private func scan(into sink: any ObservationSink, live: Bool, force: Bool = false) async throws {
        while scanning { try await Task.sleep(for: .milliseconds(5)) }
        scanning = true; defer { scanning = false }
        let started = timestamp()
        trace.record("contacts_scan_started")
        let batch: AppleBridgeContactsHistoryBatch
        do { batch = try history(token) }
        catch {
            guard authorized else { throw SourceFailure.permissionMissing }
            guard token != nil else { throw error }
            token = nil
            batch = try history(nil)
        }
        let next = batch.token
        let drop = batch.droppedEverything
        let fullScan = force || drop || token == nil || batch.requiresFullScan
        trace.record("contacts_history_fetched")
        if drop { generation = UUID().uuidString }
        var current: [String: JSON] = fullScan ? [:] : committed
        if fullScan {
        var containers: [String: Set<String>] = [:]
        for container in try store.containers(matching: nil) {
            for member in try contacts(CNContact.predicateForContactsInContainer(withIdentifier: container.identifier), identifiersOnly: true) { containers[member.identifier, default: []].insert(container.identifier) }
        }
        let fetched = try contacts()
        trace.record("contacts_fetched")
        for contact in fetched {
            let memberships = containers[contact.identifier] ?? []
            let containerID = memberships.count == 1 ? memberships.first ?? "" : ""
            current["contacts.contact/" + contact.identifier] = observation("contacts.contact", contact.identifier, try data(contact, containerID: containerID))
        }
        for group in try store.groups(matching: nil) {
            let members = try contacts(CNContact.predicateForContactsInGroup(withIdentifier: group.identifier), identifiersOnly: true)
            let value: JSON = .object(["name": .string(group.name), "member_ids": .array(members.map { .object(["$ref": .object(["collection": .string("contacts.contact"), "native_key": .string($0.identifier)])]) })])
            current["contacts.group/" + group.identifier] = observation("contacts.group", group.identifier, value)
        }
        } else {
            for key in batch.changedContactIdentifiers {
                if try isAbsent(key) { current.removeValue(forKey: "contacts.contact/" + key) }
                else { current["contacts.contact/" + key] = observation("contacts.contact", key, try data(fresh(key))) }
            }
        }
        trace.record("contacts_encoded")
        guard authorized else { throw SourceFailure.permissionMissing }
        var observations: [JSON] = []
        for key in current.keys.sorted() {
            guard let value = current[key] else { continue }
            let changed = try hash(value["data"]) != hash(committed[key]?["data"] ?? .null)
            if force || drop || token == nil || changed { observations.append(value) }
        }
        if !drop {
            let groups = fullScan ? Set(try store.groups(matching: nil).map(\.identifier)) : Set(current.values.filter { $0["collection"].string == "contacts.group" }.map { $0["native_key"].string })
            for (key, previous) in committed where current[key] == nil {
                let collection = previous["collection"].string
                if collection == "contacts.contact" { if try !isAbsent(previous["native_key"].string) { continue } }
                if collection == "contacts.group" && groups.contains(previous["native_key"].string) { continue }
                var absent = previous.object; absent["kind"] = .string("absent"); absent["revalidated"] = .bool(true); absent["observed_at"] = .string(timestamp())
                observations.append(.object(absent))
            }
        }
        let handles = Dictionary(uniqueKeysWithValues: blobs.keys.map { (String($0.dropFirst("contacts-image:".count)), $0) })
        trace.record("contacts_capture_ready")
        try await sink.emitCheckpoint(source: name, generation: generation, checkpoint: next.base64EncodedString(), observations: observations, scanID: UUID().uuidString, scanStartedAt: started, scope: ["contacts.contact", "contacts.group"], completionEvidence: "complete", blobHandles: handles, live: live)
        trace.record("contacts_capture_committed")
        token = next
        committed = drop ? committed.merging(current) { _, new in new } : current
        let baseline: JSON = .object(["checkpoint": .string(next.base64EncodedString()), "generation": .string(generation), "records": .object(committed)])
        try JSONEncoder().encode(baseline).write(to: baselineURL, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: baselineURL.path)
    }
    public func capture(_ resume: JSON, into sink: any ObservationSink) async throws {
        token = Data(base64Encoded: resume["checkpoint"].string)
        generation = resume["generation"].string.isEmpty ? UUID().uuidString : resume["generation"].string
        if let bytes = try? Data(contentsOf: baselineURL), let baseline = try? JSONDecoder().decode(JSON.self, from: bytes) {
            committed = baseline["records"].object
            if baseline["checkpoint"] != resume["checkpoint"] { token = nil }
        } else { committed = [:]; token = nil }
        let (changes, continuation) = AsyncStream<Void>.makeStream(bufferingPolicy: .bufferingNewest(1))
        let observer = NotificationCenter.default.addObserver(forName: .CNContactStoreDidChange, object: nil, queue: nil) { @Sendable [trace] _ in trace.record("contacts_notified"); continuation.yield(()) }
        let polling = Task { @ContactsActor in
            while !Task.isCancelled {
                do { try await Task.sleep(for: .seconds(2)); continuation.yield(()) } catch { return }
            }
        }
        defer { polling.cancel(); NotificationCenter.default.removeObserver(observer); continuation.finish() }
        try await scan(into: sink, live: false, force: true)
        for await _ in changes {
            try Task.checkCancellation()
            try await Task.sleep(for: .milliseconds(50))
            try await scan(into: sink, live: true)
        }
    }

    public func reconcile(into sink: any ObservationSink, key: String = "") async throws {
        try await scan(into: sink, live: true, force: true)
        if !key.isEmpty, try isAbsent(key) { try await confirmAbsence(key, into: sink) }
    }
    public func confirmAbsence(_ key: String, into sink: any ObservationSink) async throws {
        guard try isAbsent(key) else { throw SourceFailure.conflict }
        let next = try history(token).token
        let absent: JSON = .object(["kind": .string("absent"), "collection": .string("contacts.contact"), "native_key": .string(key), "scoped_alias": .string("contacts/contacts.contact/" + key), "revalidated": .bool(true), "observed_at": .string(timestamp())])
        try await sink.emitCheckpoint(source: name, generation: generation, checkpoint: next.base64EncodedString(), observations: [absent], scanID: UUID().uuidString, scanStartedAt: timestamp(), scope: ["contacts.contact"], completionEvidence: "complete", blobHandles: [:], live: true)
        token = next
    }
    public func readBlob(_ handle: String, offset: Int, length: Int) throws -> JSON {
        guard authorized, let bytes = blobs[handle], offset >= 0, offset <= bytes.count, length > 0, length <= 131072 else { throw SourceFailure.unavailable }
        let end = min(offset + length, bytes.count)
        return .object(["t": .string("BlobChunk"), "handle": .string(handle), "offset": .number(Double(offset)), "bytes": .string(bytes.subdata(in: offset..<end).base64EncodedString()), "digest": .string(hash(bytes)), "eof": .bool(end == bytes.count)])
    }
    public func prepare(_ command: JSON) async throws -> Prepared {
        guard authorized else { throw SourceFailure.permissionMissing }
        guard commandsEnabled, Self.operations.contains(command["action"]["op"].string) else { throw SourceFailure.unsupported }
        let action = command["action"]
        var before = JSON.null
        if action["op"].string == "contacts.create" {
            let key = store.defaultContainerIdentifier()
            guard !key.isEmpty, try store.containers(matching: CNContainer.predicateForContainers(withIdentifiers: [key])).count == 1 else { throw SourceFailure.unsupported }
        } else {
            before = try data(commandContact(action["native_key"].string))
            guard before == command["expected_state"] else { throw SourceFailure.conflict }
            guard before["writable"].bool else { throw SourceFailure.unsupported }
        }
        let candidate = CNMutableContact()
        try patch(candidate, action["args"])
        return Prepared(nativeState: before, fingerprint: try hash(action), action: action)
    }
    public func validate(_ prepared: Prepared) async throws {
        guard authorized else { throw SourceFailure.permissionMissing }
        if prepared.action["op"].string != "contacts.create" {
            let current = try data(commandContact(prepared.action["native_key"].string))
            guard current == prepared.nativeState else { throw SourceFailure.conflict }
            guard current["writable"].bool else { throw SourceFailure.unsupported }
        }
    }
    private func components(_ value: JSON) throws -> DateComponents? {
        if value == .null { return nil }
        guard !value.object.isEmpty, (1...12).contains(Int(value["month"].number)), (1...31).contains(Int(value["day"].number)) else { throw SourceFailure.invalid }
        var result = DateComponents()
        if let calendar = value.object["calendar"] {
            let calendars: [String: Calendar.Identifier] = ["gregorian": .gregorian, "buddhist": .buddhist, "chinese": .chinese, "coptic": .coptic, "ethiopicAmeteMihret": .ethiopicAmeteMihret, "ethiopicAmeteAlem": .ethiopicAmeteAlem, "hebrew": .hebrew, "iso8601": .iso8601, "indian": .indian, "islamic": .islamic, "islamicCivil": .islamicCivil, "japanese": .japanese, "persian": .persian, "republicOfChina": .republicOfChina, "islamicTabular": .islamicTabular, "islamicUmmAlQura": .islamicUmmAlQura]
            guard let identifier = calendars[calendar.string] else { throw SourceFailure.invalid }
            result.calendar = Calendar(identifier: identifier)
        }
        result.isLeapMonth = value.object["is_leap_month"].map { $0.bool }
        result.year = value.object["year"].map { Int($0.number) }; result.month = Int(value["month"].number); result.day = Int(value["day"].number)
        return result
    }
    private func values<T>(_ value: JSON, convert: (JSON) throws -> T) throws -> [CNLabeledValue<T>] {
        guard case .array(let entries) = value else { throw SourceFailure.invalid }
        return try entries.map { try CNLabeledValue(label: $0["label"] == .null ? nil : $0["label"].string, value: convert($0["value"])) }
    }
    private func patch(_ contact: CNMutableContact, _ args: JSON) throws {
        for (field, value) in args.object {
            if field == "id" { continue }
            if let key = Self.scalarKeys[field] {
                guard case .string(let text) = value else { throw SourceFailure.invalid }
                contact.setValue(text, forKey: key); continue
            }
            switch field {
            case "phones": contact.phoneNumbers = try values(value) { CNPhoneNumber(stringValue: $0.string) }
            case "emails": contact.emailAddresses = try values(value) { $0.string as NSString }
            case "urls": contact.urlAddresses = try values(value) { $0.string as NSString }
            case "birthday": contact.birthday = try components(value)
            case "non_gregorian_birthday": contact.nonGregorianBirthday = try components(value)
            case "dates": contact.dates = try values(value) { guard let date = try components($0) else { throw SourceFailure.invalid }; return date as NSDateComponents }
            case "relations": contact.contactRelations = try values(value) { CNContactRelation(name: $0.string) }
            case "instant_messages": contact.instantMessageAddresses = try values(value) { CNInstantMessageAddress(username: $0["username"].string, service: $0["service"].string) }
            case "social_profiles": contact.socialProfiles = try values(value) { CNSocialProfile(urlString: $0["url"].string, username: $0["username"].string, userIdentifier: $0["user_identifier"].string, service: $0["service"].string) }
            case "postal_addresses": contact.postalAddresses = try values(value) { item in
                let address = CNMutablePostalAddress()
                address.street = item["street"].string; address.subLocality = item["sub_locality"].string; address.city = item["city"].string; address.subAdministrativeArea = item["sub_administrative_area"].string; address.state = item["state"].string; address.postalCode = item["postal_code"].string; address.country = item["country"].string; address.isoCountryCode = item["iso_country_code"].string
                return address as CNPostalAddress
            }
            case "image":
                if value == .null { contact.imageData = nil }
                else { guard let image = Data(base64Encoded: value["base64"].string), image.count <= 8 * 1024 * 1024 else { throw SourceFailure.invalid }; contact.imageData = image }
            default: throw SourceFailure.invalid
            }
        }
    }
    public func dispatch(_ prepared: Prepared) async throws -> JSON {
        try await validate(prepared)
        let action = prepared.action, operation = prepared.action["op"].string
        let request = CNSaveRequest()
        let contact: CNMutableContact
        if operation == "contacts.create" { contact = CNMutableContact(); try patch(contact, action["args"]); request.add(contact, toContainerWithIdentifier: store.defaultContainerIdentifier()) }
        else {
            guard let current = try commandContact(action["native_key"].string).mutableCopy() as? CNMutableContact else { throw SourceFailure.corrupt }
            contact = current
            if operation == "contacts.delete" { request.delete(contact) }
            else { try patch(contact, action["args"]); request.update(contact) }
        }
        try store.execute(request)
        if operation == "contacts.delete" { guard try isAbsent(contact.identifier) else { throw SourceFailure.unavailable } }
        else { guard try data(fresh(contact.identifier)) == data(contact) else { throw SourceFailure.unavailable } }
        return .object(["native_key": .string(contact.identifier), "observed": .bool(true), "absent": .bool(operation == "contacts.delete")])
    }
    public func resolve(_ command: String, evidence: JSON) async -> JSON {
        guard authorized else { return .null }
        let action = evidence["prepared"]["action"]
        let key = action["native_key"].string.isEmpty ? evidence["native_key"].string : action["native_key"].string
        guard !key.isEmpty else { return .null }
        do {
            if action["op"].string == "contacts.delete" { guard try isAbsent(key) else { return .null } }
            else {
                let actual = try fresh(key)
                guard let desired = actual.mutableCopy() as? CNMutableContact else { return .null }
                try patch(desired, action["args"])
                guard try data(actual) == data(desired) else { return .null }
            }
            return .object(["native_key": .string(key), "observed": .bool(true), "absent": .bool(action["op"].string == "contacts.delete"), "prepared": evidence["prepared"]])
        } catch { return .null }
    }
}
