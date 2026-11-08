# Hosted product domains and parallel migration

Status: Accepted for implementation; live qualification pending.

Apple Bridge uses applebridge.eilmer.si for relay control and newly enrolled Macs,
app.applebridge.eilmer.si for hosted sign-in, and <id>.applebridge.eilmer.si for
customer gateways. eilmer.si serves the product website at /applebridge and its
installer at /applebridge/install. Only exact configured SNI names terminate at
the relay; customer TLS continues to terminate on the Mac.

Retain ab.lexaai.tech, app.ab.lexaai.tech, old device records, credentials, account
storage and the old sign-in RP in parallel. A new enrollment zone does not rename
existing device records. Explicit legacy zones permit reopening their unchanged
state; they do not admit arbitrary suffixes. The existing custom-domain journey
can migrate a Mac after DNS ownership proof and certificate issuance, keeping its
original device identity. Existing iPhone pairing, signed paths, salts and bundle
identities remain unchanged. Do not silently reset credentials or enrollment.

The two sign-in origins require separate Better Auth configurations and RP IDs,
with the same securely retained accounts database and secret. Keep the old origin
available. New-origin passkeys require enrollment there; old RP credentials cannot
be transferred. Sessions and invite cookies use Secure, HttpOnly, Path=/ and the
__Host- prefix without Domain. Never enable parent-domain cookies or wildcard
trusted origins: customer hosts must not read, set or shadow sign-in cookies.

Use separate loopback sign-in instances with their own public URLs. Control
serves two origin views inside one process, sharing its sole writer, fence and
policy synchronization mutex; never start two writers against the control DB.
Validate concurrent account SQLite WAL access and RP lookup before production.
Public ports expose only the relay. Back up state securely before deployment,
retain the newest state on rollback, and keep both DNS zones and certificate
renewals available. Additional SNI routes share bounded admission capacity and
have separate ACME certificate storage. DNS publication selects the configured
provider by exact suffix boundary.

Future projects receive separate product subdomains, sign-in RPs, secrets,
services and tenant namespaces. Paid plans, billing, account creation changes,
quotas and cross-product identity are deferred to a future ADR and explicit
owner authorization. Domain migration does not imply implementing those systems.
