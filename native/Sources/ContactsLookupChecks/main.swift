import Foundation
import Contacts
import ContactsSource
import SourceSupport

@main
struct ContactsLookupChecks {
    static func main() throws {
        var reads = 0
        let contact = CNMutableContact()
        contact.givenName = "[ab-test] lookup"
        let found = try ContactsSource.existingContact(fetch: { reads += 1; return contact }, authorized: { true })
        guard reads == 1, found === contact else { throw SourceFailure.corrupt }
        let missing = NSError(domain: CNErrorDomain, code: CNError.Code.recordDoesNotExist.rawValue)
        let absent = try ContactsSource.existingContact(fetch: { reads += 1; throw missing }, authorized: { true })
        guard reads == 2, absent == nil else { throw SourceFailure.corrupt }
        do {
            _ = try ContactsSource.existingContact(fetch: { throw missing }, authorized: { false })
            throw SourceFailure.corrupt
        } catch SourceFailure.permissionMissing {}
        let failure = NSError(domain: "[ab-test] transient", code: 1)
        do {
            _ = try ContactsSource.existingContact(fetch: { throw failure }, authorized: { true })
            throw SourceFailure.corrupt
        } catch let error as NSError {
            guard error == failure else { throw error }
        }
        let otherDomain = NSError(domain: "[ab-test] different", code: CNError.Code.recordDoesNotExist.rawValue)
        do {
            _ = try ContactsSource.existingContact(fetch: { throw otherDomain }, authorized: { true })
            throw SourceFailure.corrupt
        } catch let error as NSError {
            guard error == otherDomain else { throw error }
        }
        print("single-fetch Contacts lookup and error handling: pass")
    }
}
