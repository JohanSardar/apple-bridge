# Contacts Me card and notes

`contacts.contact.is_me` identifies the unified Contacts Me card using `CNContactStore.unifiedMeContactWithKeys(toFetch:)`. It is null when the Me card cannot be resolved. Captures refresh all contacts when the resolved Me identifier changes so the old card loses its marker. The existing `contacts.list` read model exposes this field.

CON-ME compares the single marked record to independent AppleTruth using SHA-256 of the unified identifier. Evidence contains only a boolean match and identifier digest, never names. It does not alter the Me card.

Thirty-second recovery polls compare the current Contacts history token with the last committed fetch-result token and check the Me identifier. Unchanged polls skip history enumeration. Contacts notifications retain their 50 ms coalescing delay; command reconciliation remains immediate. The fallback recovers missed notifications without continuously fetching the Me card. Notifications, forced captures and explicit command reconciliation retain their history fetch path; only fetch-result tokens advance the persisted checkpoint.

Notes remain `note: unavailable`. Reading or writing `CNContactNoteKey` requires the restricted `com.apple.developer.contacts.notes` entitlement, absent from the Helper signing entitlements. The key is excluded from fetches and notes writes are not exposed. CON-NOTE checks the installed signed Helper entitlements and the unavailable record marker; its status is limited, not proof of notes read/write support.
