# Isolated R1 control service ↔ forwarder port

Control worker owns `rust/crates/ab-relay-control`, its new SQLite database, and this R1 contract. Forwarder worker retains all legacy JSON state, credentials, relay/1, owner allowlist and enrollment behaviour. Do not activate this policy on existing routes during the format freeze. Production activation waits for Phase 8 and R5.

The service is built only with `--features isolated-control` and starts only with `AB_RELAY_CONTROL_ISOLATED=1`. It binds loopback only. Browser Host, Origin, RP ID and cookie host are exactly `app.lexaai.tech`. No DNS credentials or account secrets belong on a forwarder.

New isolated policy API (Rust library; no legacy wire changes):

- `Control::admit(device, credential, generation, unix_seconds)` returns an opaque 30-second stream lease. Check before accepting every stream; unavailable/fenced/unknown/suspended/revoked policy denies admission. Credential hashes stay in control storage. Generation fences old tunnel owners after rotation, suspension, resume or revoke.
- `Control::charge(lease, upstream_bytes, downstream_bytes, unix_seconds)` reserves both directions atomically against device and account monthly and per-second limits. Call before forwarding bytes, in chunks at most 1 MiB; denial means no forwarding. Charges renew the lease for 30 seconds. Never continue with a cached policy or expired lease. Idle streams must renew through a zero-byte charge or close before expiration.
- `Control::release(lease)` closes a stream. Leases also expire after crashes.
- Browser suspend/revoke/rotate commits new device generation and deletes leases before returning a disconnect notification. Forwarder integration must terminate sockets for that device after the commit; all subsequent charge/admission attempts already fail closed. `pending_disconnects`/`ack_disconnect` supplies a durable retryable outbox.

Limits: 32 concurrent streams per device, 64 per account; 1,048,576 bytes per second per device and account; 10,737,418,240 bytes per device and account per UTC month. The rate window is a UTC second, not a sliding token bucket. Both traffic directions count. No payload inspection. Global live counts can be measured by the forwarder; no new guessed global limit is imposed here.

The SQLite writer mutex serializes policy and usage operations. File locks exclude a second process. A separate fsynced epoch file is advanced before safety mutations; SQLite commits the same epoch with the policy. Mismatch or missing fence fails closed. The fence must live outside database backup/restore scope on a durable monotonic storage boundary. Losing or rolling back both the fence and database requires external operational fencing; a local file cannot detect rollback of its entire host. Never automatically reset/recreate an existing database's missing fence.

Run local fixture scenarios on the build host: `cargo test -p ab-relay-control --all-features --test scenarios`. These are isolated Rust service scenarios, not evidence for legacy forwarder parity. No real GitHub app, DNS, Mac permissions or live VM is needed.

## Separate process transport

An optional **separate loopback policy listener** is enabled only by `AB_RELAY_CONTROL_POLICY_TOKEN` (base64url encoding of at least 32 random bytes). `AB_RELAY_CONTROL_POLICY_LISTEN` defaults to `127.0.0.1:8790`. A forwarder uses this narrowly scoped policy credential, never a GitHub credential or hosted session. Every request is POST JSON with `Authorization: Bearer <policy token>`:

| Endpoint | JSON fields | Result |
| --- | --- | --- |
| `/v1/policy/admit` | device, credential, generation | lease, expires |
| `/v1/policy/charge` | lease, upstream, downstream | allowed |
| `/v1/policy/release` | lease | released |
| `/v1/policy/disconnects` | none | ordered events with id, device, epoch |
| `/v1/policy/ack` | id | acknowledged |

The policy listener is private service transport; do not proxy it to `app.` or to the tunnel domain. Forwarder disconnect workers acknowledge only after closing affected sockets. HTTP failures and malformed replies deny forwarding. This isolated implementation is not wired into the production forwarder.

## Operator setup and privacy

Configuration: `AB_RELAY_CONTROL_DB` points to a new database, `AB_RELAY_CONTROL_FENCE` to a separate append-only fsynced epoch journal outside database backups; `AB_RELAY_CONTROL_LISTEN` defaults to `127.0.0.1:8789`. GitHub credentials come only from `AB_RELAY_GITHUB_CLIENT_ID` and `AB_RELAY_GITHUB_CLIENT_SECRET`. The app callback must be `https://app.lexaai.tech/v1/oauth/callback`; requested OAuth scopes are empty, and responses carrying extra scopes are refused. Only `github:<immutable numeric id>` is retained. No emails or logins are used for identity linking.

With the service stopped and isolation flag set, `ab-relay-control --create-invite <expiry-unix-seconds> <use-count>` prints a new invite once to the operator's stdout. This is intentional secret delivery, not a log. It shares the exclusive writer lock with the server. Invite and new account allocation are one SQLite transaction, reserving a two-device account; completing a grant atomically consumes the pending transaction and its device slot. Revoked 24-hex names remain tombstones.

Service passkeys support ES256 WebAuthn with none attestation, required user verification, exact RP hash and exact HTTPS origin, single-use server challenges and credential counter checking. Eight random recovery codes are issued when adding a passkey, invalidate previous codes, and are stored only as hashes. Recovery consumes a code once and invalidates previous sessions. No platform authenticator prompts occur in scenarios: an in-memory P-256 authenticator supplies registration and assertion fixtures.

No request bodies, URLs, credentials, account identifiers or IP addresses are logged. Startup errors are fixed redacted strings; credential-bearing state has redacted Debug implementations. Source throttles retain only hashed source addresses and bounded rolling windows. Account deletion clears sessions, credentials, pending grants and usage, retaining revoked device tombstones and a pseudonymous deleted-account row. The control service can see immutable account subjects, device public keys, tunnel ownership, traffic totals and timing; it does not receive tunnel payloads or Mac access grants. Existing owner credentials remain in the frozen legacy store; grandfathering that live device is a post-freeze activation/migration operation, not performed by this isolated service.

## Scenario acceptance map

`cargo test -p ab-relay-control --features isolated-control --test scenarios` runs the following isolated fixture journeys (test names use lowercase underscores):

| Plan scenario | Acceptance covered |
| --- | --- |
| RELAY-ACCOUNT-ENROLL | immutable subject reuse, browser-bound one-use OAuth state, ES256 service passkey registration/login, assertion replay refusal, one-use recovery |
| RELAY-DEVICE-GRANT | displayed code/fingerprint, slot/key binding, wrong-key refusal, one-time completion, 30-second fresh proof, 10-minute expiry, polling backoff, source throttling |
| RELAY-INVITE-ONLY | uninvited refusal, invite exhaustion, 25-account ceiling, two-device slots, exclusive writer |
| RELAY-ACCOUNT-REVOKE | cross-account refusal, suspend/resume, fresh-key rotation, credential/generation fencing, durable revocation before disconnect outbox acknowledgement, local policy invalidation under one second |
| RELAY-QUOTA-ACCOUNT | device/account concurrency, bidirectional aggregate rate and monthly caps, UTC-month reset |
| RELAY-LOG-REDACTION | seeded request/config secrets absent from error responses, Debug and startup stderr, isolation flag refusal, exact Host and Origin refusals |
| RELAY-RESTORE-FENCE | backup before revoke, ordinary restart, restored stale database rejected against external durable epoch |

Supplementary race and transport cases check two simultaneous completions competing for one slot and unauthenticated policy RPC refusal. Browser rendering and real forwarder socket-disconnect latency are integration gates for the forwarder worker, not claims made by these fixtures. No legacy parity scenario is replaced or weakened.

API and cryptographic references: [GitHub OAuth](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps), [minimal scopes](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/scopes-for-oauth-apps), [WebAuthn](https://www.w3.org/TR/webauthn-3/), [RFC 8628](https://www.rfc-editor.org/rfc/rfc8628).
