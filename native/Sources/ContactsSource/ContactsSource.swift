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
    private var generation = UUID().uuidString
    private var blobs: [String: Data] = [:]
    private var meIdentifier: String?
    private var queuedPhotos: Set<String> = []
    private var photoReady: AsyncStream<Void>.Continuation?
    private var photoResults: [String: MediaResult] = [:]
    private var variantURLs: [String: URL] = [:]
    private let photoInputDirectory: URL
    private static let groupOperations = ["contacts.create_group", "contacts.rename_group", "contacts.delete_group", "contacts.add_member", "contacts.remove_member"]
    private static let operations = ["contacts.create", "contacts.update", "contacts.delete"]
    private static let scalarKeys = ["given_name": CNContactGivenNameKey, "middle_name": CNContactMiddleNameKey, "family_name": CNContactFamilyNameKey, "previous_family_name": CNContactPreviousFamilyNameKey, "nickname": CNContactNicknameKey, "name_prefix": CNContactNamePrefixKey, "name_suffix": CNContactNameSuffixKey, "phonetic_given_name": CNContactPhoneticGivenNameKey, "phonetic_middle_name": CNContactPhoneticMiddleNameKey, "phonetic_family_name": CNContactPhoneticFamilyNameKey, "phonetic_organization": CNContactPhoneticOrganizationNameKey, "organization": CNContactOrganizationNameKey, "job_title": CNContactJobTitleKey, "department": CNContactDepartmentNameKey]
    private static var keys: [any CNKeyDescriptor] {
        (Array(scalarKeys.values) + [CNContactIdentifierKey, CNContactTypeKey, CNContactPhoneNumbersKey, CNContactEmailAddressesKey, CNContactPostalAddressesKey, CNContactUrlAddressesKey, CNContactBirthdayKey, CNContactNonGregorianBirthdayKey, CNContactDatesKey, CNContactSocialProfilesKey, CNContactInstantMessageAddressesKey, CNContactRelationsKey, CNContactImageDataKey, CNContactThumbnailImageDataKey, CNContactImageDataAvailableKey]).map { $0 as NSString }
    }

    public nonisolated init(home: URL, commandsEnabled: Bool = false) throws {
        self.commandsEnabled = commandsEnabled
        photoInputDirectory = home.appendingPathComponent("contacts-photo-inputs", isDirectory: true)
    }

    private var authorized: Bool { CNContactStore.authorizationStatus(for: .contacts) == .authorized }

    public func status() async -> JSON? {
        .object(["authorized": .bool(authorized), "proven_groups": .array(groupCapabilities().map(JSON.string))])
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
    private func containerExists(_ key: String) throws -> Bool {
        guard authorized else { throw SourceFailure.permissionMissing }
        return try !store.containers(matching: CNContainer.predicateForContainers(withIdentifiers: [key])).isEmpty
    }
    private func readEverything(_ metadata: [String: JSON]) throws -> [String: JSON] {
        var current: [String: JSON] = [:]
        var containers: [String: Set<String>] = [:]
        for container in try store.containers(matching: nil) {
            let value: JSON = .object(["identifier": .string(container.identifier), "name": .string(container.name), "type": .number(Double(container.type.rawValue)), "writable": .bool(container.identifier == store.defaultContainerIdentifier())])
            current["contacts.container/" + container.identifier] = observation("contacts.container", container.identifier, value)
            for member in try contacts(CNContact.predicateForContactsInContainer(withIdentifier: container.identifier), identifiersOnly: true) { containers[member.identifier, default: []].insert(container.identifier) }
        }
        let fetched = try contacts()
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
        return current
    }
    private func blobHandles() -> [String: String] {
        var handles = Dictionary(uniqueKeysWithValues: blobs.keys.map { (String($0.dropFirst("contacts-image:".count)), $0) })
        for handle in variantURLs.keys { handles[String(handle.dropFirst("media:".count))] = handle }
        return handles
    }
    public func enumerate() async throws -> SourceEnumeration {
        guard authorized else { throw SourceFailure.permissionMissing }
        let activity = ForegroundActivity.shared.begin()
        defer { ForegroundActivity.shared.end(activity) }
        meIdentifier = try? store.unifiedMeContactWithKeys(toFetch: [CNContactIdentifierKey as NSString]).identifier
        let current = try readEverything([:])
        guard authorized else { throw SourceFailure.permissionMissing }
        return SourceEnumeration(generation: generation, rows: current.keys.sorted().compactMap { current[$0] }, handles: blobHandles())
    }
    public func revalidate(_ identities: [JSON]) async throws -> [String] {
        guard authorized else { throw SourceFailure.permissionMissing }
        var states: [String] = []
        for identity in identities {
            let key = identity["native_key"].string
            switch identity["collection"].string {
            case "contacts.contact":
                states.append(try isAbsent(key) ? RevalidationState.missing : RevalidationState.present)
            case "contacts.container":
                states.append(try containerExists(key) ? RevalidationState.present : RevalidationState.missing)
            case "contacts.group":
                let groups = try store.groups(matching: CNGroup.predicateForGroups(withIdentifiers: [key]))
                states.append(groups.isEmpty ? RevalidationState.missing : RevalidationState.present)
            default: throw SourceFailure.invalid
            }
            guard authorized else { throw SourceFailure.permissionMissing }
        }
        return states
    }


    public func serves(_ handle: String) -> Bool { variantURLs[handle] != nil }
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
        return .object(["t": .string("BlobChunk"), "handle": .string(handle), "offset": .number(Double(offset)), "bytes": .string(bytes.subdata(in: offset..<end).base64EncodedString()), "digest": .string(String(handle.dropFirst("contacts-image:".count))), "eof": .bool(end == bytes.count)])
    }
    public func photosReady() -> AsyncStream<Void> {
        let (stream, continuation) = AsyncStream<Void>.makeStream(bufferingPolicy: .bufferingNewest(1))
        photoReady = continuation
        return stream
    }
    public func derivePhoto(_ digest: String) -> JSON {
        if blobs["contacts-image:" + digest] != nil, photoResults[digest] == nil, queuedPhotos.insert(digest).inserted {
            Task { await self.makePhoto(digest) }
        }
        return .object([:])
    }
    private func makePhoto(_ digest: String) async {
        defer { queuedPhotos.remove(digest) }
        guard let bytes = blobs["contacts-image:" + digest] else { return }
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
        guard let result = try? await withTaskCancellationHandler(operation: { try await work.value }, onCancel: { work.cancel() }), !Task.isCancelled else { return }
        photoResults[digest] = result
        photoReady?.yield(())
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

    private var groupProofURL: URL { FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Application Support/AppleBridgeDev/contacts-group-proofs.json") }
    private func groupCapabilities() -> [String] {
        guard let bytes = try? Data(contentsOf: groupProofURL), let value = try? JSONDecoder().decode(JSON.self, from: bytes), value["os_build"].string == ProcessInfo.processInfo.operatingSystemVersionString, value["container_id"].string == store.defaultContainerIdentifier(), value["cleaned"].bool else { return [] }
        return Self.groupOperations.filter { op in value["operations"].array.contains(.string(op)) }
    }
    private func requireGroupCapability(_ operation: String) throws {
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
    public func proveGroups() async -> JSON {
        guard authorized else { return .object(["state": .string("limited"), "reason": .string("contacts_full_access_required")]) }
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
                verification = "dispatch_and_read_back"
                let result = try writeGroup(action)
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


    public func primitive(_ operation: String, args request: JSON) async throws -> JSON {
        guard authorized else { throw SourceFailure.permissionMissing }
        guard commandsEnabled else { throw SourceFailure.unsupported }
        let key = request["native_key"].string
        switch operation {
        case "contacts.read_contact":
            meIdentifier = try? store.unifiedMeContactWithKeys(toFetch: [CNContactIdentifierKey as NSString]).identifier
            do { return .object(["exists": .bool(true), "data": try data(commandContact(key))]) }
            catch SourceFailure.conflict { return .object(["exists": .bool(false)]) }
        case "contacts.can_create":
            let container = store.defaultContainerIdentifier()
            let matching = container.isEmpty ? 0 : try store.containers(matching: CNContainer.predicateForContainers(withIdentifiers: [container])).count
            return .object(["allowed": .bool(matching == 1)])
        case "contacts.check_patch":
            try patch(CNMutableContact(), request["args"])
            return .object([:])
        case "contacts.group_capabilities":
            return .object(["operations": .array(groupCapabilities().map(JSON.string))])
        case "contacts.read_group":
            if request["observer"].bool {
                let observer = CNContactStore()
                guard let found = try observer.groups(matching: CNGroup.predicateForGroups(withIdentifiers: [key])).first else { return .object(["exists": .bool(false)]) }
                let members = try observer.unifiedContacts(matching: CNContact.predicateForContactsInGroup(withIdentifier: key), keysToFetch: [CNContactIdentifierKey as NSString])
                return .object(["exists": .bool(true), "data": .object(["name": .string(found.name), "member_ids": .array(members.sorted { $0.identifier < $1.identifier }.map { .object(["$ref": .object(["collection": .string("contacts.contact"), "native_key": .string($0.identifier)])]) })])])
            }
            do { return .object(["exists": .bool(true), "data": try groupData(group(key))]) }
            catch SourceFailure.conflict { return .object(["exists": .bool(false)]) }
        case "contacts.goal":
            let actual = try fresh(key)
            guard let desired = actual.mutableCopy() as? CNMutableContact else { throw SourceFailure.corrupt }
            try patch(desired, request["args"])
            return .object(["actual": try data(actual), "desired": try data(desired)])
        case "contacts.write":
            return try await writeContact(request)
        case "contacts.write_group":
            return try writeGroup(request)
        default:
            throw SourceFailure.unsupported
        }
    }

    private func writeContact(_ request: JSON) async throws -> JSON {
        let operation = request["op"].string
        meIdentifier = try? store.unifiedMeContactWithKeys(toFetch: [CNContactIdentifierKey as NSString]).identifier
        guard Self.operations.contains(operation) else { throw SourceFailure.unsupported }
        let save = CNSaveRequest()
        let contact: CNMutableContact
        if operation == "contacts.create" { contact = CNMutableContact(); try patch(contact, request["args"]); save.add(contact, toContainerWithIdentifier: store.defaultContainerIdentifier()) }
        else {
            guard let current = try commandContact(request["native_key"].string).mutableCopy() as? CNMutableContact else { throw SourceFailure.corrupt }
            contact = current
            if operation == "contacts.delete" { save.delete(contact) }
            else { try patch(contact, request["args"]); save.update(contact) }
        }
        try store.execute(save)
        let deleted = operation == "contacts.delete"
        let exists = !(try isAbsent(contact.identifier))
        let saved: JSON = deleted ? .null : try data(contact)
        let observed: JSON = exists && !deleted ? try data(fresh(contact.identifier)) : .null
        return .object(["native_key": .string(contact.identifier), "exists_after": .bool(exists), "saved": saved, "observed": observed])
    }

    private func writeGroup(_ request: JSON) throws -> JSON {
        let operation = request["op"].string
        guard Self.groupOperations.contains(operation) else { throw SourceFailure.unsupported }
        let save = CNSaveRequest()
        let mutable: CNMutableGroup
        if operation == "contacts.create_group" {
            mutable = CNMutableGroup(); mutable.name = request["args"]["name"].string
            save.add(mutable, toContainerWithIdentifier: store.defaultContainerIdentifier())
        } else {
            guard let value = try group(request["native_key"].string).mutableCopy() as? CNMutableGroup else { throw SourceFailure.corrupt }
            mutable = value
            switch operation {
            case "contacts.rename_group": mutable.name = request["args"]["name"].string; save.update(mutable)
            case "contacts.delete_group": save.delete(mutable)
            default:
                let contact = try commandContact(request["list_key"].string)
                if operation == "contacts.add_member" { save.addMember(contact, to: mutable) }
                else { save.removeMember(contact, from: mutable) }
            }
        }
        try store.execute(save)
        return .object(["native_key": .string(mutable.identifier)])
    }
}
