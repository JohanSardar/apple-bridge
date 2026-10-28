# Contacts Me card and notes

`contacts.contact.is_me` identifies the unified Contacts Me card using `CNContactStore.unifiedMeContactWithKeys(toFetch:)`. It is null when the Me card cannot be resolved. Captures refresh all contacts when the resolved Me identifier changes so the old card loses its marker. The existing `contacts.list` read model exposes this field.

CON-ME compares the single marked record to independent AppleTruth using SHA-256 of the unified identifier. Evidence contains only a boolean match and identifier digest, never names. It does not alter the Me card.

Notes remain `note: unavailable`. Reading or writing `CNContactNoteKey` requires the restricted `com.apple.developer.contacts.notes` entitlement, absent from the Helper signing entitlements. The key is excluded from fetches and notes writes are not exposed. CON-NOTE checks the installed signed Helper entitlements and the unavailable record marker; its status is limited, not proof of notes read/write support.
