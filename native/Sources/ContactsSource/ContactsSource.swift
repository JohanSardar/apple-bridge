import Foundation
@preconcurrency import Contacts
import CryptoKit
import ContactsHistory
import MediaDerivatives
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
    private var groupSelfTesting = false
    private var groupSelfTestCreatedKey = ""
    private var meIdentifier: String?
    private var photoContinuation: AsyncStream<String>.Continuation?
    private var queuedPhotos: Set<String> = []
    private var photoResults: [String: MediaResult] = [:]
    private var variantURLs: [String: URL] = [:]
    private let photoInputDirectory: URL
    private let baselineURL: URL
    private static let groupOperations = ["contacts.create_group", "contacts.rename_group", "contacts.delete_group", "contacts.add_member", "contacts.remove_member"]
    private static let operations = ["contacts.create", "contacts.update", "contacts.delete"]
    public nonisolated static let coverage: JSON = .object(["fields": .string("partial"), "missing": .array([.string("note: unavailable without restricted entitlement"), .string("writability: conservatively limited to default container"), .string("contact_poster: unavailable through public Contacts API on this SDK")])])
    private static let scalarKeys = ["given_name": CNContactGivenNameKey, "middle_name": CNContactMiddleNameKey, "family_name": CNContactFamilyNameKey, "previous_family_name": CNContactPreviousFamilyNameKey, "nickname": CNContactNicknameKey, "name_prefix": CNContactNamePrefixKey, "name_suffix": CNContactNameSuffixKey, "phonetic_given_name": CNContactPhoneticGivenNameKey, "phonetic_middle_name": CNContactPhoneticMiddleNameKey, "phonetic_family_name": CNContactPhoneticFamilyNameKey, "phonetic_organization": CNContactPhoneticOrganizationNameKey, "organization": CNContactOrganizationNameKey, "job_title": CNContactJobTitleKey, "department": CNContactDepartmentNameKey]
    private static var keys: [any CNKeyDescriptor] {
        (Array(scalarKeys.values) + [CNContactIdentifierKey, CNContactTypeKey, CNContactPhoneNumbersKey, CNContactEmailAddressesKey, CNContactPostalAddressesKey, CNContactUrlAddressesKey, CNContactBirthdayKey, CNContactNonGregorianBirthdayKey, CNContactDatesKey, CNContactSocialProfilesKey, CNContactInstantMessageAddressesKey, CNContactRelationsKey, CNContactImageDataKey, CNContactThumbnailImageDataKey, CNContactImageDataAvailableKey]).map { $0 as NSString }
    }

    public nonisolated init(home: URL, commandsEnabled: Bool = false, trace: CaptureTrace = CaptureTrace()) throws {
        self.trace = trace
        self.commandsEnabled = commandsEnabled
        baselineURL = home.appendingPathComponent("contacts-baseline.json")
        photoInputDirectory = home.appendingPathComponent("contacts-photo-inputs", isDirectory: true)
    }

    private var authorized: Bool { CNContactStore.authorizationStatus(for: .contacts) == .authorized }

    public func open() async -> SourceReadiness {
        var coverage = Self.coverage.object
        let proven = groupCapabilities()
        coverage["operations"] = .array(Self.groupOperations.map { .object(["operation": .string($0), "proven": .bool(proven.contains($0)), "reason": .string(proven.contains($0) ? "proven_public_contacts_save_request" : "os_build_group_proof_required")]) })
        return SourceReadiness(state: authorized ? "ready" : "permission_missing", coverage: .object(coverage), capabilities: authorized && commandsEnabled ? (Self.operations + groupCapabilities()) : [], reason: authorized ? "" : "contacts_full_access_required")
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
        fields["is_me"] = meIdentifier.map { .bool(contact.identifier == $0) } ?? .null
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
            if photoResults[digest] == nil { photoResults[digest] = MediaDerivatives.cached(digest) }
            if let result = photoResults[digest] { try attachPhoto(result, to: &fields) }
            else if queuedPhotos.insert(digest).inserted { photoContinuation?.yield(digest) }
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
    private func scan(into sink: any ObservationSink, live: Bool, force: Bool = false, notificationAt: UInt64 = 0) async throws {
        let scanID = UUID().uuidString
        let metadata: [String: JSON] = ["scan_id": .string(scanID), "notification_ns": .integer(notificationAt)]
        trace.record("contacts_scan_requested", fields: metadata)
        while scanning { try await Task.sleep(for: .milliseconds(5)) }
        var completed = false
        scanning = true
        defer {
            scanning = false
            trace.record(completed ? "contacts_scan_finished" : "contacts_scan_abandoned", fields: metadata)
        }
        let started = timestamp()
        trace.record("contacts_scan_started", fields: metadata)
        let batch: AppleBridgeContactsHistoryBatch
        do { batch = try history(token) }
        catch {
            guard authorized else { throw SourceFailure.permissionMissing }
            guard token != nil else { throw error }
            trace.record("contacts_history_reset", fields: metadata)
            token = nil
            batch = try history(nil)
        }
        let next = batch.token
        let drop = batch.droppedEverything
        let fullScan = force || drop || token == nil || batch.requiresFullScan
        trace.record("contacts_history_fetched", fields: metadata.merging(["changed_groups": .number(Double(batch.changedGroupIdentifiers.count)), "full_scan": .bool(fullScan), "token_changed": .bool(next != token), "forced": .bool(force), "dropped_everything": .bool(drop), "initial_token": .bool(token == nil), "full_scan_event_kinds": .array(batch.fullScanEventKinds.map(JSON.string))]) { _, new in new })
        guard fullScan || next != token else { completed = true; return }
        let activity = ForegroundActivity.shared.begin()
        defer { ForegroundActivity.shared.end(activity) }
        if drop { generation = UUID().uuidString }
        meIdentifier = try? store.unifiedMeContactWithKeys(toFetch: [CNContactIdentifierKey as NSString]).identifier
        var current: [String: JSON] = fullScan ? [:] : committed
        if fullScan {
        var containers: [String: Set<String>] = [:]
        for container in try store.containers(matching: nil) {
            let value: JSON = .object(["identifier": .string(container.identifier), "name": .string(container.name), "type": .number(Double(container.type.rawValue)), "writable": .bool(container.identifier == store.defaultContainerIdentifier())])
            current["contacts.container/" + container.identifier] = observation("contacts.container", container.identifier, value)
            for member in try contacts(CNContact.predicateForContactsInContainer(withIdentifier: container.identifier), identifiersOnly: true) { containers[member.identifier, default: []].insert(container.identifier) }
        }
        let fetched = try contacts()
        trace.record("contacts_fetched", fields: metadata)
        for contact in fetched {
            let memberships = containers[contact.identifier] ?? []
            let containerID = memberships.count == 1 ? memberships.first ?? "" : ""
            current["contacts.contact/" + contact.identifier] = observation("contacts.contact", contact.identifier, try data(contact, containerID: containerID))
        }
        for group in try store.groups(matching: nil) {
            let members = try contacts(CNContact.predicateForContactsInGroup(withIdentifier: group.identifier), identifiersOnly: true)
            let value = try groupData(group, members: members)
            current["contacts.group/" + group.identifier] = observation("contacts.group", group.identifier, value)
        }
        } else {
            for key in batch.changedContactIdentifiers {
                if try isAbsent(key) { current.removeValue(forKey: "contacts.contact/" + key) }
                else {
                    let contact = try fresh(key)
                    trace.record("contacts_contact_fetched", fields: metadata)
                    current["contacts.contact/" + key] = observation("contacts.contact", key, try data(contact))
                }
            }
            if !batch.changedGroupIdentifiers.isEmpty {
                let changedGroups = try store.groups(matching: CNGroup.predicateForGroups(withIdentifiers: batch.changedGroupIdentifiers))
                let byID = Dictionary(uniqueKeysWithValues: changedGroups.map { ($0.identifier, $0) })
                for key in batch.changedGroupIdentifiers {
                    guard let group = byID[key] else { current.removeValue(forKey: "contacts.group/" + key); continue }
                    let members = try contacts(CNContact.predicateForContactsInGroup(withIdentifier: group.identifier), identifiersOnly: true)
                    let value = try groupData(group, members: members)
                    current["contacts.group/" + key] = observation("contacts.group", key, value)
                }
            }
        }
        trace.record("contacts_encoded", fields: metadata)
        guard authorized else { throw SourceFailure.permissionMissing }
        var observations: [JSON] = []
        for key in current.keys.sorted() {
            guard let value = current[key] else { continue }
            let changed = value["data"] != (committed[key]?["data"] ?? .null)
            if force || drop || token == nil || changed { observations.append(value) }
        }
        trace.record("contacts_compared", fields: metadata)
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
        var handles = Dictionary(uniqueKeysWithValues: blobs.keys.map { (String($0.dropFirst("contacts-image:".count)), $0) })
        for handle in variantURLs.keys { handles[String(handle.dropFirst("media:".count))] = handle }
        trace.record("contacts_capture_ready", fields: metadata.merging(["empty": .bool(observations.isEmpty)]) { _, new in new })
        try await sink.emitCheckpoint(source: name, generation: generation, checkpoint: next.base64EncodedString(), observations: observations, scanID: scanID, scanStartedAt: started, scope: ["contacts.contact", "contacts.group", "contacts.container"], completionEvidence: "complete", blobHandles: handles, live: live)
        trace.record("contacts_capture_committed", fields: metadata)
        token = next
        committed = drop ? committed.merging(current) { _, new in new } : current
        let baseline: JSON = .object(["checkpoint": .string(next.base64EncodedString()), "generation": .string(generation), "records": .object(committed)])
        try JSONEncoder().encode(baseline).write(to: baselineURL, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: baselineURL.path)
        completed = true
    }
    public func capture(_ resume: JSON, into sink: any ObservationSink) async throws {
        token = Data(base64Encoded: resume["checkpoint"].string)
        generation = resume["generation"].string.isEmpty ? UUID().uuidString : resume["generation"].string
        if let bytes = try? Data(contentsOf: baselineURL), let baseline = try? JSONDecoder().decode(JSON.self, from: bytes) {
            committed = baseline["records"].object
            if baseline["checkpoint"] != resume["checkpoint"] { token = nil }
        } else { committed = [:]; token = nil }
        let (photoJobs, photoSink) = AsyncStream<String>.makeStream()
        photoContinuation = photoSink
        queuedPhotos = []
        let photoWorker = Task { @ContactsActor in
            for await digest in photoJobs {
                guard !Task.isCancelled else { return }
                await makePhoto(digest, into: sink)
            }
        }
        defer { photoSink.finish(); photoContinuation = nil; photoWorker.cancel() }
        let (changes, continuation) = AsyncStream<UInt64>.makeStream(bufferingPolicy: .bufferingNewest(1))
        let observer = NotificationCenter.default.addObserver(forName: .CNContactStoreDidChange, object: nil, queue: nil) { @Sendable [trace] _ in let delivered = CaptureTrace.now(); trace.record("contacts_notified", fields: ["delivered_ns": .integer(delivered)]); continuation.yield(delivered) }
        let polling = Task { @ContactsActor in
            while !Task.isCancelled {
                do { try await Task.sleep(for: .seconds(2)); continuation.yield(0) } catch { return }
            }
        }
        defer { polling.cancel(); NotificationCenter.default.removeObserver(observer); continuation.finish() }
        try await scan(into: sink, live: false, force: true)
        for await notified in changes {
            try Task.checkCancellation()
            try await Task.sleep(for: .milliseconds(50))
            try await scan(into: sink, live: true, notificationAt: notified)
        }
    }

    public func reconcile(into sink: any ObservationSink, key: String = "") async throws {
        let wasGroup = committed["contacts.group/" + key] != nil
        try await scan(into: sink, live: true)
        if !key.isEmpty, !wasGroup, try store.groups(matching: CNGroup.predicateForGroups(withIdentifiers: [key])).isEmpty, try isAbsent(key) { try await confirmAbsence(key, into: sink) }
    }
    public func confirmAbsence(_ key: String, into sink: any ObservationSink) async throws {
        guard try isAbsent(key) else { throw SourceFailure.conflict }
        let next = try history(token).token
        let absent: JSON = .object(["kind": .string("absent"), "collection": .string("contacts.contact"), "native_key": .string(key), "scoped_alias": .string("contacts/contacts.contact/" + key), "revalidated": .bool(true), "observed_at": .string(timestamp())])
        try await sink.emitCheckpoint(source: name, generation: generation, checkpoint: next.base64EncodedString(), observations: [absent], scanID: UUID().uuidString, scanStartedAt: timestamp(), scope: ["contacts.contact"], completionEvidence: "complete", blobHandles: [:], live: true)
        token = next
    }
    public func readBlob(_ handle: String, offset: Int, length: Int) throws -> JSON {
        guard authorized else { throw SourceFailure.permissionMissing }
        if let url = variantURLs[handle] {
            guard offset >= 0, length > 0, length <= 131072 else { throw SourceFailure.invalid }
            let file = try FileHandle(forReadingFrom: url)
            defer { try? file.close() }
            let size = try file.seekToEnd()
            guard UInt64(offset) <= size else { throw SourceFailure.invalid }
            try file.seek(toOffset: UInt64(offset))
            let bytes = try file.read(upToCount: length) ?? Data()
            return .object(["t": .string("BlobChunk"), "handle": .string(handle), "offset": .number(Double(offset)), "bytes": .string(bytes.base64EncodedString()), "digest": .string(String(handle.dropFirst("media:".count))), "eof": .bool(UInt64(offset + bytes.count) == size)])
        }
        guard let bytes = blobs[handle], offset >= 0, offset <= bytes.count, length > 0, length <= 131072 else { throw SourceFailure.unavailable }
        let end = min(offset + length, bytes.count)
        return .object(["t": .string("BlobChunk"), "handle": .string(handle), "offset": .number(Double(offset)), "bytes": .string(bytes.subdata(in: offset..<end).base64EncodedString()), "digest": .string(hash(bytes)), "eof": .bool(end == bytes.count)])
    }
    private func attachPhoto(_ result: MediaResult, to fields: inout [String: JSON]) throws {
        var derivatives: [String: JSON] = [:]
        for (name, variant) in result.derivatives {
            var value = try JSONDecoder().decode(JSON.self, from: JSONEncoder().encode(variant)).object
            value.removeValue(forKey: "file")
            value["state"] = .string("present")
            derivatives[name] = .object(value)
            variantURLs[variant.handle] = MediaDerivatives.url(variant)
        }
        fields["derivatives"] = .object(derivatives)
        fields["media"] = try JSONDecoder().decode(JSON.self, from: JSONEncoder().encode(result.media))
    }
    private func makePhoto(_ digest: String, into sink: any ObservationSink) async {
        defer { queuedPhotos.remove(digest) }
        guard let bytes = blobs["contacts-image:" + digest] else { return }
        let expectedGeneration = generation
        let input = photoInputDirectory.appendingPathComponent(digest + ".image")
        let work = Task.detached(executorPreference: MediaExecutor.shared, priority: .utility) {
            try await ForegroundActivity.shared.waitUntilIdle(for: .milliseconds(250))
            try FileManager.default.createDirectory(at: input.deletingLastPathComponent(), withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            try bytes.write(to: input, options: .atomic)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: input.path)
            defer { try? FileManager.default.removeItem(at: input) }
            let activity = MediaWork(digest: digest, checkpoint: { quiet in
                try await ForegroundActivity.shared.waitUntilIdle(for: quiet)
                while MediaDerivatives.paused { try await Task.sleep(for: .seconds(1)); try Task.checkCancellation() }
            }, interruption: { callback in
                let id = ForegroundActivity.shared.interrupt(callback)
                return { ForegroundActivity.shared.removeInterruption(id) }
            }, trace: { _ in })
            return try await MediaWork.$current.withValue(activity) { try await MediaDerivatives.make(input, digest: digest) }
        }
        do {
            let result = try await withTaskCancellationHandler { try await work.value } onCancel: { work.cancel() }
            while scanning { try await Task.sleep(for: .milliseconds(5)) }
            guard !Task.isCancelled, authorized, expectedGeneration == generation else { return }
            scanning = true
            defer { scanning = false }
            photoResults[digest] = result
            var observations: [JSON] = []
            var changed: [String: JSON] = [:]
            for (key, previous) in committed where previous["collection"].string == "contacts.contact" && previous["data"]["image"]["digest"].string == digest {
                var fields = previous["data"].object
                try attachPhoto(result, to: &fields)
                let value = observation("contacts.contact", previous["native_key"].string, .object(fields))
                changed[key] = value
                observations.append(value)
            }
            guard !observations.isEmpty, let token else { return }
            var handles: [String: String] = [:]
            for variant in result.derivatives.values { handles[variant.digest] = variant.handle }
            try await sink.emitCheckpoint(source: name, generation: generation, checkpoint: token.base64EncodedString(), observations: observations, scanID: UUID().uuidString, scanStartedAt: timestamp(), scope: ["contacts.contact"], completionEvidence: "", blobHandles: handles, live: false)
            committed.merge(changed) { _, new in new }
            let baseline: JSON = .object(["checkpoint": .string(token.base64EncodedString()), "generation": .string(generation), "records": .object(committed)])
            try JSONEncoder().encode(baseline).write(to: baselineURL, options: .atomic)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: baselineURL.path)
        } catch { trace.record("contacts_photo_deferred") }
    }
    public func prepare(_ command: JSON) async throws -> Prepared {
        guard authorized else { throw SourceFailure.permissionMissing }
        guard commandsEnabled, (Self.operations + Self.groupOperations).contains(command["action"]["op"].string) else { throw SourceFailure.unsupported }
        let action = command["action"]
        if Self.groupOperations.contains(action["op"].string) { return try prepareGroup(command) }
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
        if Self.groupOperations.contains(prepared.action["op"].string) { try validateGroup(prepared); return }
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
        if Self.groupOperations.contains(prepared.action["op"].string) { return try await dispatchGroup(prepared) }
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
        trace.record("contacts_effect_done", keys: [contact.identifier], fields: ["operation": .string(operation)])
        if operation == "contacts.delete" { guard try isAbsent(contact.identifier) else { throw SourceFailure.unavailable } }
        else { guard try data(fresh(contact.identifier)) == data(contact) else { throw SourceFailure.unavailable } }
        return .object(["native_key": .string(contact.identifier), "observed": .bool(true), "absent": .bool(operation == "contacts.delete")])
    }
    public func resolve(_ command: String, evidence: JSON) async -> JSON {
        guard authorized else { return .null }
        let action = evidence["prepared"]["action"]
        if Self.groupOperations.contains(action["op"].string) { return resolveGroup(evidence) }
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

    private var groupProofURL: URL { FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/AppleBridgeDev/contacts-group-proofs.json") }
    private func groupCapabilities() -> [String] {
        guard let bytes = try? Data(contentsOf: groupProofURL), let value = try? JSONDecoder().decode(JSON.self, from: bytes), value["os_build"].string == ProcessInfo.processInfo.operatingSystemVersionString, value["container_id"].string == store.defaultContainerIdentifier(), value["cleaned"].bool else { return [] }
        return Self.groupOperations.filter { op in value["operations"].array.contains(.string(op)) }
    }
    private func requireGroupCapability(_ operation: String) throws {
        if groupSelfTesting { return }
        guard groupCapabilities().contains(operation) else { throw SourceFailure.capabilityUnproven }
    }
    private func group(_ key: String) throws -> CNGroup {
        guard let result = try store.groups(matching: CNGroup.predicateForGroups(withIdentifiers: [key])).first else { throw SourceFailure.conflict }
        return result
    }
    private func groupData(_ group: CNGroup, members: [CNContact]? = nil) throws -> JSON {
        let containers = try store.containers(matching: CNContainer.predicateForContainerOfGroup(withIdentifier: group.identifier))
        let container = containers.count == 1 ? containers[0].identifier : ""
        let contacts = try members ?? self.contacts(CNContact.predicateForContactsInGroup(withIdentifier: group.identifier), identifiersOnly: true)
        return .object(["name": .string(group.name), "container_id": .string(container), "writable": .bool(!container.isEmpty && container == store.defaultContainerIdentifier()), "member_ids": .array(contacts.sorted { $0.identifier < $1.identifier }.map { .object(["$ref": .object(["collection": .string("contacts.contact"), "native_key": .string($0.identifier)])]) })])
    }
    private func prepareGroup(_ command: JSON) throws -> Prepared {
        let action = command["action"]
        try requireGroupCapability(action["op"].string)
        let before: JSON
        if action["op"].string == "contacts.create_group" {
            let key = store.defaultContainerIdentifier()
            guard !key.isEmpty, try store.containers(matching: CNContainer.predicateForContainers(withIdentifiers: [key])).count == 1 else { throw SourceFailure.unsupported }
            before = .null
        } else {
            before = try groupData(group(action["native_key"].string))
            guard before == command["expected_state"] else { throw SourceFailure.conflict }
            guard before["writable"].bool else { throw SourceFailure.unsupported }
        }
        let prepared = Prepared(nativeState: before, fingerprint: try hash(action), action: action)
        try validateGroup(prepared)
        return prepared
    }
    private func validateGroup(_ prepared: Prepared) throws {
        let action = prepared.action, op = action["op"].string
        try requireGroupCapability(op)
        if op != "contacts.create_group" {
            let current = try groupData(group(action["native_key"].string))
            guard current == prepared.nativeState else { throw SourceFailure.conflict }
            guard current["writable"].bool else { throw SourceFailure.unsupported }
        }
        if op == "contacts.create_group" || op == "contacts.rename_group" {
            let name = action["args"]["name"].string
            guard !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, name.utf8.count <= 256 else { throw SourceFailure.invalid }
        }
        if op == "contacts.add_member" || op == "contacts.remove_member" {
            let contact = try commandContact(action["list_key"].string)
            guard try container(contact) == prepared.nativeState["container_id"].string else { throw SourceFailure.unsupported }
        }
    }
    private func groupEffect(_ action: JSON, key: String) throws -> Bool {
        let op = action["op"].string
        let observer = CNContactStore()
        let groups = try observer.groups(matching: CNGroup.predicateForGroups(withIdentifiers: [key]))
        if op == "contacts.delete_group" { return groups.isEmpty }
        guard let current = groups.first else { return false }
        if op == "contacts.create_group" || op == "contacts.rename_group" { return current.name == action["args"]["name"].string }
        let members = try observer.unifiedContacts(matching: CNContact.predicateForContactsInGroup(withIdentifier: key), keysToFetch: [CNContactIdentifierKey as NSString])
        return members.contains { $0.identifier == action["list_key"].string } == (op == "contacts.add_member")
    }
    private func dispatchGroup(_ prepared: Prepared) async throws -> JSON {
        let action = prepared.action, op = prepared.action["op"].string
        let request = CNSaveRequest()
        let mutable: CNMutableGroup
        if op == "contacts.create_group" {
            mutable = CNMutableGroup(); mutable.name = action["args"]["name"].string
            request.add(mutable, toContainerWithIdentifier: store.defaultContainerIdentifier())
        } else {
            guard let value = try group(action["native_key"].string).mutableCopy() as? CNMutableGroup else { throw SourceFailure.corrupt }
            mutable = value
            switch op {
            case "contacts.rename_group": mutable.name = action["args"]["name"].string; request.update(mutable)
            case "contacts.delete_group": request.delete(mutable)
            case "contacts.add_member", "contacts.remove_member":
                let contact = try commandContact(action["list_key"].string)
                if op == "contacts.add_member" { request.addMember(contact, to: mutable) }
                else { request.removeMember(contact, from: mutable) }
            default: throw SourceFailure.unsupported
            }
        }
        try store.execute(request)
        if groupSelfTesting && op == "contacts.create_group" { groupSelfTestCreatedKey = mutable.identifier }
        var observed = false
        for _ in 0..<20 {
            if try groupEffect(action, key: mutable.identifier) { observed = true; break }
            try await Task.sleep(for: .milliseconds(50))
        }
        guard observed else { throw SourceFailure.unavailable }
        return .object(["native_key": .string(mutable.identifier), "observed": .bool(true), "absent": .bool(op == "contacts.delete_group")])
    }
    private func resolveGroup(_ evidence: JSON) -> JSON {
        let action = evidence["prepared"]["action"]
        let key = action["native_key"].string.isEmpty ? evidence["native_key"].string : action["native_key"].string
        guard !key.isEmpty, (try? groupEffect(action, key: key)) == true else { return .null }
        return .object(["native_key": .string(key), "observed": .bool(true), "absent": .bool(action["op"].string == "contacts.delete_group"), "prepared": evidence["prepared"]])
    }
    public func proveGroups() async -> JSON {
        guard authorized else { return .object(["state": .string("limited"), "reason": .string("contacts_full_access_required")]) }
        guard !groupSelfTesting else { return .object(["state": .string("limited"), "reason": .string("proof_already_running")]) }
        groupSelfTesting = true
        groupSelfTestCreatedKey = ""
        defer { groupSelfTesting = false }
        let tag = "[ab-test " + UUID().uuidString + "]"
        let contact = CNMutableContact()
        contact.givenName = tag; contact.organizationName = "Bridge Test"
        var contactCreated = false
        var groupKey = ""
        var steps: [JSON] = []
        var operation = "contacts.create_fixture"
        var verification = "save"
        func cleanup() throws {
            if !groupKey.isEmpty, let owned = try store.groups(matching: CNGroup.predicateForGroups(withIdentifiers: [groupKey])).first {
                guard owned.name.hasPrefix("Bridge Test " + tag) else { throw SourceFailure.conflict }
                let members = try contacts(CNContact.predicateForContactsInGroup(withIdentifier: groupKey), identifiersOnly: true)
                let ownedContactKey = contact.identifier
                guard members.allSatisfy({ $0.identifier == ownedContactKey }) else { throw SourceFailure.conflict }
                guard let mutable = owned.mutableCopy() as? CNMutableGroup else { throw SourceFailure.corrupt }
                let request = CNSaveRequest(); request.delete(mutable); try store.execute(request)
            }
            if contactCreated, !((try? isAbsent(contact.identifier)) ?? false) {
                guard let mutable = try fresh(contact.identifier).mutableCopy() as? CNMutableContact, mutable.organizationName == "Bridge Test", mutable.givenName == tag else { throw SourceFailure.conflict }
                let request = CNSaveRequest(); request.delete(mutable); try store.execute(request)
            }
        }
        do {
            let request = CNSaveRequest(); request.add(contact, toContainerWithIdentifier: store.defaultContainerIdentifier()); try store.execute(request)
            contactCreated = true
            for op in Self.groupOperations.filter({ $0 != "contacts.delete_group" }) + ["contacts.add_member", "contacts.delete_group"] {
                operation = op
                var args: [String: JSON] = [:]
                if op == "contacts.create_group" || op == "contacts.rename_group" { args["name"] = .string("Bridge Test " + tag + (op == "contacts.rename_group" ? " renamed" : "")) }
                let action: JSON = .object(["op": .string(op), "native_key": .string(groupKey), "list_key": .string(contact.identifier), "args": .object(args)])
                let expected = groupKey.isEmpty ? JSON.null : try groupData(group(groupKey))
                let prepared = try prepareGroup(.object(["action": action, "expected_state": expected]))
                verification = "dispatch_and_read_back"
                let result = try await dispatch(prepared)
                groupKey = result["native_key"].string
                verification = "independent_read_back"
                guard try groupEffect(action, key: groupKey) else { throw SourceFailure.unavailable }
                verification = "contact_preserved"
                guard try !isAbsent(contact.identifier) else { throw SourceFailure.unavailable }
                steps.append(.object(["operation": .string(op), "proven": .bool(true)]))
            }
            try cleanup()
            guard try isAbsent(contact.identifier), try store.groups(matching: CNGroup.predicateForGroups(withIdentifiers: [groupKey])).isEmpty else { throw SourceFailure.unavailable }
            let proof: JSON = .object(["os_build": .string(ProcessInfo.processInfo.operatingSystemVersionString), "operations": .array(Self.groupOperations.map(JSON.string)), "container_id": .string(store.defaultContainerIdentifier()), "route": .string("CNSaveRequest"), "cleaned": .bool(true)])
            try FileManager.default.createDirectory(at: groupProofURL.deletingLastPathComponent(), withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            try JSONEncoder().encode(proof).write(to: groupProofURL, options: .atomic)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: groupProofURL.path)
            return .object(["state": .string("proven"), "steps": .array(steps), "cleaned": .bool(true)])
        } catch {
            let reason = String(describing: error)
            if groupKey.isEmpty { groupKey = groupSelfTestCreatedKey }
            var diagnostic = JSON.null
            if operation == "contacts.remove_member", !groupKey.isEmpty {
                do {
                    let rawRequest = CNContactFetchRequest(keysToFetch: [CNContactIdentifierKey as NSString])
                    rawRequest.unifyResults = false
                    rawRequest.predicate = CNContact.predicateForContactsInGroup(withIdentifier: groupKey)
                    var rawKeys: [String] = []
                    try CNContactStore().enumerateContacts(with: rawRequest) { value, _ in rawKeys.append(value.identifier) }
                    let rawCount = rawKeys.count
                    let rawMatches = rawKeys.contains(contact.identifier)
                    let request = CNSaveRequest()
                    request.removeMember(contact, from: try group(groupKey))
                    try store.execute(request)
                    let action: JSON = .object(["op": .string(operation), "list_key": .string(contact.identifier)])
                    diagnostic = .object(["raw_member_count_after_unified_remove": .number(Double(rawCount)), "raw_member_matches_created": .bool(rawMatches), "original_contact_remove_observed": .bool(try groupEffect(action, key: groupKey))])
                } catch { diagnostic = .object(["reason": .string(String(describing: error))]) }
            }
            let cleaned: Bool
            do { try cleanup(); cleaned = true } catch { cleaned = false }
            try? FileManager.default.removeItem(at: groupProofURL)
            return .object(["state": .string("limited"), "reason": .string(reason), "operation": .string(operation), "verification": .string(verification), "diagnostic": diagnostic, "steps": .array(steps), "cleaned": .bool(cleaned)])
        }
    }

}
