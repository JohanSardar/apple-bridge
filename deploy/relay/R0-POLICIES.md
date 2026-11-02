# Owner-only relay policy drafts

These drafts describe the owner-only deployment. Review the contact method, actual log settings and retention automation before inviting another account.

## Abuse response draft

Admission is restricted to operator-approved device keys. A report must identify the affected hostname and describe the behavior, with the minimum supporting material necessary. The operator investigates reports without fetching arbitrary reported URLs on a privileged host. Confirmed abuse leads to durable suspension followed by tunnel disconnection; permanent revocation retains a hostname tombstone. Appeals and emergency response coverage need a published contact method before invitations. No response-time promise is made before coverage exists.

## Privacy draft

The relay routes encrypted traffic and stores device public keys, random hostnames, credential hashes, policy and aggregate byte usage. The forwarding process observes network addresses, routing names, connection timing and traffic volume. Passthrough does not require application plaintext. A provisional retention target is 24 hours for raw source addresses, 90 days for necessary administrative and usage events, and 30 days for backups. These are proposed limits, not verified deletion automation. Deletion must remove ordinary account/device records while retaining only the minimum permanent revocation tombstones and explicitly justified incident records. Do not record bearer credentials, private keys, feed payloads or tokenized URLs in logs or reports.

## Transparency draft

The operator controls relay routing and currently holds DNS authority used for DNS-01 certificate issuance. That authority can enable a certificate splice of legacy relay/1 replica traffic. Pairing signatures alone do not bind relay/1 to its outer TLS connection. Sensitive replicas must use direct pinned mTLS over a private route without public fallback, or remain paused. Browser clients using hosted names still trust the operator's effective DNS and certificate authority. Narrowing DNS privileges reduces scope but does not remove this trust. Moving DNS authority off forwarding VMs and introducing channel binding are later phases. The synthetic splice scenario documents the vulnerable baseline, not evidence of a past incident.
