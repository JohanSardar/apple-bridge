import Foundation
@preconcurrency import Contacts
import CryptoKit

extension AppleTruth {
    @MainActor
    static func contacts(_ verb: String, _ request: [String: Any]) throws -> [String: Any] {
        guard CNContactStore.authorizationStatus(for: .contacts) == .authorized else { return ["status": "unverified", "reason": "contacts_access_required"] }
        let store = CNContactStore()
        let keys = [CNContactIdentifierKey, CNContactGivenNameKey, CNContactMiddleNameKey, CNContactFamilyNameKey, CNContactPreviousFamilyNameKey, CNContactNamePrefixKey, CNContactNameSuffixKey, CNContactNicknameKey, CNContactPhoneticGivenNameKey, CNContactPhoneticMiddleNameKey, CNContactPhoneticFamilyNameKey, CNContactPhoneticOrganizationNameKey, CNContactOrganizationNameKey, CNContactJobTitleKey, CNContactDepartmentNameKey, CNContactTypeKey, CNContactPhoneNumbersKey, CNContactEmailAddressesKey, CNContactPostalAddressesKey, CNContactUrlAddressesKey, CNContactBirthdayKey, CNContactNonGregorianBirthdayKey, CNContactDatesKey, CNContactSocialProfilesKey, CNContactInstantMessageAddressesKey, CNContactRelationsKey, CNContactImageDataKey, CNContactThumbnailImageDataKey, CNContactImageDataAvailableKey]
        func read() throws -> [CNContact] {
            let fetch = CNContactFetchRequest(keysToFetch: keys.map { $0 as NSString })
            fetch.unifyResults = true
            var contacts: [CNContact] = []
            try store.enumerateContacts(with: fetch) { contact, _ in contacts.append(contact) }
            return contacts
        }
        let tag = request["tag"] as? String ?? ""
        let action = request["action"] as? String ?? ""
        let initial = try read()
        var effectAt = DispatchTime.now().uptimeNanoseconds
        func owned(_ contact: CNContact) -> Bool { contact.organizationName == "Bridge Test" && !tag.isEmpty && contact.givenName.hasPrefix(tag + " ") }
        if verb == "fixture" {
            guard tag.hasPrefix("[ab-test "), tag.hasSuffix("]"), tag.count > 11, !tag.contains("\n") else { return ["status": "unverified", "reason": "invalid_test_tag"] }
            let save = CNSaveRequest()
            if action == "create" {
                guard !initial.contains(where: owned) else { return ["status": "unverified", "reason": "fixture_already_exists"] }
                let contact = CNMutableContact()
                contact.givenName = tag + " Testperson"; contact.familyName = "Testfamily"; contact.organizationName = "Bridge Test"
                contact.middleName = "Testmiddle"; contact.nickname = "Testnick"; contact.namePrefix = "Mx"; contact.nameSuffix = "Test"
                contact.phoneticGivenName = "Test"; contact.phoneticMiddleName = "Test"; contact.phoneticFamilyName = "Test"; contact.phoneticOrganizationName = "Test"
                contact.jobTitle = "Testjob"; contact.departmentName = "Testdepartment"
                contact.phoneNumbers = [CNLabeledValue(label: CNLabelPhoneNumberMobile, value: CNPhoneNumber(stringValue: "+15555550123"))]
                contact.emailAddresses = [CNLabeledValue(label: CNLabelHome, value: "bridge-test@example.com" as NSString)]
                contact.urlAddresses = [CNLabeledValue(label: CNLabelHome, value: "https://example.com" as NSString)]
                let address = CNMutablePostalAddress(); address.street = "1 Test Street"; address.city = "Testville"; address.state = "TS"; address.postalCode = "00000"; address.country = "Testland"; address.isoCountryCode = "US"
                contact.postalAddresses = [CNLabeledValue(label: CNLabelHome, value: address as CNPostalAddress)]
                contact.birthday = DateComponents(year: 2000, month: 1, day: 2)
                contact.dates = [CNLabeledValue(label: CNLabelDateAnniversary, value: DateComponents(year: 2020, month: 3, day: 4) as NSDateComponents)]
                contact.contactRelations = [CNLabeledValue(label: CNLabelContactRelationFriend, value: CNContactRelation(name: "Testfriend"))]
                contact.socialProfiles = [CNLabeledValue(label: CNLabelHome, value: CNSocialProfile(urlString: "https://example.com/test", username: "test", userIdentifier: "test", service: "test"))]
                contact.instantMessageAddresses = [CNLabeledValue(label: CNLabelHome, value: CNInstantMessageAddress(username: "test@example.com", service: "test"))]
                save.add(contact, toContainerWithIdentifier: store.defaultContainerIdentifier())
            } else {
                let matches = initial.filter(owned)
                if action != "cleanup" && matches.count != 1 { return ["status": "unverified", "reason": "fixture_not_unique"] }
                for contact in matches {
                    guard let mutable = contact.mutableCopy() as? CNMutableContact else { return ["status": "unverified", "reason": "immutable_fixture"] }
                    if action == "cleanup" || action == "delete" { save.delete(mutable) }
                    else if action == "modify" { mutable.jobTitle = request["job_title"] as? String ?? "Updated Testjob"; save.update(mutable) }
                    else if action == "image" {
                        guard let bytes = Data(base64Encoded: request["base64"] as? String ?? ""), bytes.count < 1_048_576 else { return ["status": "unverified", "reason": "invalid_fixture_image"] }
                        mutable.imageData = bytes; save.update(mutable)
                    } else { return ["status": "unverified", "reason": "invalid_fixture_action"] }
                }
            }
            try store.execute(save)
            effectAt = DispatchTime.now().uptimeNanoseconds
        } else if verb != "observe" { return ["status": "unverified", "reason": "invalid_verb"] }
        let all = try read()
        let tagged = all.filter(owned)
        var coverage: [String: Int] = [:]
        for key in keys {
            coverage[key] = all.filter { contact in
                guard let value = contact.value(forKey: key) else { return false }
                if let text = value as? String { return !text.isEmpty }
                if let values = value as? [Any] { return !values.isEmpty }
                return true
            }.count
        }
        let records: [[String: Any]] = tagged.map { contact in
            var result: [String: Any] = ["given_name": contact.givenName, "family_name": contact.familyName, "organization": contact.organizationName, "job_title": contact.jobTitle, "phone_count": contact.phoneNumbers.count, "email_count": contact.emailAddresses.count]
            if let bytes = contact.imageData { result["image_digest"] = SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined() }
            return result
        }
        return ["status": "verified", "total_count": all.count, "bridge_test_count": all.filter { $0.organizationName == "Bridge Test" }.count, "tagged_count": tagged.count, "records": records, "coverage_counts": coverage, "effect_uptime_ns": effectAt]
    }
}
