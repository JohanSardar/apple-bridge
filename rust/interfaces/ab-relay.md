> Go oracle retired: the Go gateway, its parity oracles and the Go lab were deleted. Go commands, file names and Go-versus-Rust results below record what was verified while Go existed; the stored evidence files are unchanged. The Rust tests and `make rust-ci` are the gate now.

# Production Go → Rust relay cutover runbook

Status: **procedure only; not executed**. Target: the existing production relay service, with its existing address, DNS, owner allowlist, credentials and frozen relay/1 JSON state. This swaps the legacy forwarder only; it does not activate isolated account/control policy or relay/2.

1. **Gate and stage.** Require green `make rust-ci` and `make lint`, the eight relay sims, mixed Go/Rust/legacy relay/1 interoperability, and the real public TLS/DNS/ACME/OAuth/MCP/60-second SSE scenarios on an isolated test relay. A private test CA proves transport but does not satisfy public ACME or DNS gates. Stop if any required gate is blocked. Build the Linux Rust `ab-relay` from the approved commit on `ssh build`; record commit and SHA-256, stage under a versioned binary path, and verify the checksum on the destination. Keep the currently running Go executable at a separate versioned path.
2. **Capture exact configuration.** Record `systemctl cat ab-relay`, resolved executable, service UID/GID, environment-file paths and flags privately. Preserve the current `-addr`, `-control-host`, `-zone`, `-state`, `-owner-keys-file`, DNS project/zone, idle and budget values, control ACME and optional APNs configuration. Rust accepts the existing single-dash flags. Verify its service identity can read owner keys, TLS/ACME cache and configured credentials, and can write the state directory. Do not copy secrets to tickets, git or logs. Do not introduce a new default or omit the R0 owner gate.
3. **Stop and snapshot.** Arrange a brief reconnect interruption. Stop `ab-relay`; confirm its PID exited and listeners closed. Never run Go and Rust concurrently against one JSON file. Take a private, consistent backup of the entire state directory (including control certificate/account cache), configuration and owner keys, preserving ownership and modes. Record the backup location. Keep DNS and the public address unchanged.
4. **Swap and start.** Atomically install a service override that changes only the executable path to the staged Rust binary, retaining every argument and environment setting. Run `systemctl daemon-reload`, then start `ab-relay`. Check service health, expected listeners and private state permissions. Confirm existing clients reconnect without re-enrollment; run synthetic Go, Rust and legacy relay/1 probes through distinct test routes, verify origin TLS, OAuth/MCP and at least 60 seconds of SSE. Confirm unknown keys fail admission, usage increases, budget enforcement and existing revocation tombstones survive. Inspect redacted errors and reconnect failures. Do not place calls or send messages.
5. **Rollback immediately on failure.** Stop Rust and confirm exit. Preserve its current state and logs privately for diagnosis. Restore the prior executable override and exact configuration, then daemon-reload and start Go **against the current JSON state**: parity supports both directions, and retaining it preserves new revocations, rotations and byte charges. Never blindly restore the pre-cutover snapshot: doing so can resurrect revoked credentials and erase usage. If Go cannot read the current state, keep service stopped and repair under explicit fencing before resuming; use the backup only after reconciling all safety mutations. Verify existing-client reconnect and a synthetic HTTPS/SSE request. No client home migration or DNS rollback is needed.

Test-host reproduction: use a fresh directory on `ssh build` with `go run ./lab/testing/relayrust <ab-relay> <remote_client> --test-deploy <new-directory>`. The runner binds control `127.0.0.1:18443` and ingress `127.0.0.1:19443`, uses `relay.test`, a private CA, synthetic keys and separate Go/Rust/old-client homes. Deployment mode uses the production-default two-minute idle budget and a 90-second bulk-transfer deadline; ordinary parity mode retains its two-second idle budget and 15-second transfer deadline. It retains private state for inspection and stops its relay processes on exit. No public DNS or production gateway is involved.

---

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

## R2 certificate protections contract

Owner: R2 certificate implementation. R1 and ab-relay workers consume this contract; no forwarder DNS dependency is introduced.

## Gates

Compile `ab-remote`, `ab-runtime`, and `ab-relay-control` with `r2-certificates`. All features default off. Mac opt-in additionally requires `remote/r2-certificates.json` containing `true`; control requires `caa::Control::new(adapter, true)`. Existing relay/1 enrollment and proofs are unchanged.

## Enrollment and CAA

CAA types live in `ab_relay_control::caa`; R1 retains the crate-root `Control` API. The separate control service calls `caa::Control::apply` at enrollment with a `Change::Enroll`, revision zero, and `enrolled_mac` obtained from its authenticated host enrollment record (never from the request body). The caller must authorize ownership of `binding.host` before passing the key. A Mac signs `Change::transcript()` using its Ed25519 identity. This covers operation, host, account URI, both keys, revision, and expiry; the proof domain is `apple-bridge-caa/2`. Maximum validity is five minutes. Recovery uses the recovery public key pinned in the preceding binding, never a relay assertion. Rotation uses the preceding Mac key. Either operation can replace both keys and the ACME account. Store the recovery private key off the relay.

`DnsAdapter::commit(previous, next, records)` is a durable compare-and-swap transaction: reject conflicting revisions and CNAME hosts; replace the full per-host CAA RRset and persist the next binding together, or report failure without advancing the binding. An external DNS provider needs a durable intent/outbox and retries with read-back before enrollment is acknowledged; no concrete provider is installed by this change. Replaying a completed revision is refused. Serialize host updates in the service, including recovery. Never expose this adapter, its credentials, or an RPC that can call it to the forwarders. The separate control-service bootstrap must install this interface; the existing Go relay DNS routes remain legacy until that worker removes them.

The only emitted records are flags zero, `issue` with `letsencrypt.org; accounturi=<Mac URI>; validationmethods=dns-01`, and `issuewild` with `;`. No alternate CA or wildcard authorization is emitted. User DNS strong mode requires A/AAAA records; a CNAME moves effective CAA to its target. Authoritative DNS operators can replace CAA; the fixture explicitly demonstrates this residual risk.

## Mac CT

`ct::run` is independent of relay control responses and runs alongside the tunnel reconnect loop. It queries public crt.sh over HTTPS for the assigned name, the configured custom domain, and all names in `remote/ct-names.json` (a JSON string array). Include every custom name there when enrollment changes. Each owned name also queries its parent wildcard. DER parsing retains poisoned precertificates, and SHA-256 is computed locally. Locally installed certificates add fingerprints to `remote/ct-known.json`, independent of relay acknowledgment. Known precertificate fingerprints can be recorded by the Mac issuance integration with `ct::remember`; final and precertificate DER hashes differ and are deliberately never inferred equivalent.

`CtClient` is injectable for offline fixtures. Product `PublicCt` has a fixed public HTTPS origin, bounded responses, per-request and per-query timeouts, no redirects, and no relay all-clear input. Aggregation always has an explicit lag warning: a successful scan means observed results, never proof of no issuance. Query failures retain prior unexpected findings. A missing report or age over 15 minutes appears as unknown in `ct::doctor`, wired into runtime doctor. Findings are sticky until a Mac-local fingerprint acknowledgment.

`IphonePush::alert` is the asynchronous iPhone delivery hook. The default `Notifications` implementation persists `remote/ct-alert.json` and emits the warning to the process notification/log path. The iPhone worker must implement this trait using its authenticated encrypted push delivery and inject it into `ct::run`; the default relay adapter does not claim APNs delivery. Doctor reads `remote/ct-monitor.json` even if push fails. No CT result is allowed to clear relay warnings.

## Offline verification

On build host: `CARGO_TARGET_DIR=~/targets/relay-r2 cargo test -p ab-remote -p ab-relay-control --features r2-certificates --locked` from `rust/`. The `relay_ct_independent` and `relay_caa_account` integration scenarios exercise Rust product entry points with fixture adapters. The registered `bridge-lab` sim scenarios `lab/scenarios/core/RELAY-CT-INDEPENDENT.yaml` and `lab/scenarios/core/RELAY-CAA-ACCOUNT.yaml` invoke these fixture scenarios offline, require named acceptance assertions, and record `impl=rust`. Run both from the repository root with `go run ./cmd/bridge-lab run lab/scenarios/core/RELAY-CT-INDEPENDENT.yaml lab/scenarios/core/RELAY-CAA-ACCOUNT.yaml --level sim --json` on the Linux build host. Cargo dependencies must already be cached; the runner refuses non-Linux and non-sim execution. This is component fixture qualification through bridge-lab, not live gateway qualification. No production DNS, CA, or relay is contacted. Pebble/Docker were absent on the build host, so the CAA scenario uses an RFC 8657 enforcing ACME authorization fixture with CNAME traversal and wildcard precedence. Sources: [RFC 8657](https://www.rfc-editor.org/rfc/rfc8657.html), [RFC 8659](https://www.rfc-editor.org/rfc/rfc8659.html), [crt.sh](https://crt.sh/).


---

The following section describes the frozen legacy forwarder port. It does not activate the isolated R1/R2 control policy on existing routes.

# ab-relay R1 parity contract

The `ab-relay` crate and binary implement the frozen Go relay/1 server. No accounts, SQL migration, relay/2 or production activation belong to this port.

## Ownership and persistence

`Store` owns the legacy hostname-keyed JSON map, challenge consumption, Ed25519 verification, admission, policy and durable byte reservation. Device field names and omission rules match `internal/relay/store.go`. New credentials contain 32 random bytes in unpadded base64url; only their SHA-256 hex digest is stored. New hostnames retain 24 hex digits. Writes use a private temporary file, file fsync, atomic rename and parent-directory fsync. Failed saves restore the in-memory record. Rotation retains limits, usage, disabled state, aliases and certificate history. Revocation is sticky.

The owner allowlist is reloaded after every enrollment signature verification. An unreadable, invalid, empty or oversized configured file denies admission without modifying state. Source throttling uses the socket address, ignores forwarded headers, shares the enrollment/rotation allowance, and bounds the source map at 4096 entries.

## Boundaries

`Server` owns public admission, session replacement, cancellation, inactivity and durable bidirectional pacing. `hello` owns the bounded TLS ClientHello parser. `mux` owns smux v2 framing over binary WebSocket messages; it must interoperate with `github.com/xtaci/smux` v1.5.57, including peer receive windows, health frames and stream cancellation.

`http::DnsProvider::txt` is the injectable signed DNS publication boundary. Tests must supply a mock and never use real GCP. Domain registration requires both the origin and ACME challenge CNAMEs before persisting the alias. Certificate status is best-effort CT aggregator monitoring, preserving the Go limitation. Legacy relay/1 remains vulnerable to the recorded TLS splice; the port must not silently change its signed transcript.

## Verification requirements

Only scenarios actually driving the Rust binary may claim `impl=rust`. The lab-only `lab/relayfixture` adapter drives the Rust `lab_fixture` executable when `AB_RELAY_FIXTURE` is set, and keeps the Go backend otherwise. Scenario assertions remain in the existing relay scenarios. Callback adapters supply local DNS publication, CNAME resolution and CT responses; the shipped binary has no fixture RPC endpoints. `RELAY-TLS-SPLICE` now routes its origin leg through the selected forwarder while preserving the vulnerable baseline and all three rejection controls.

`lab/testing/relayrust` drives the shipped binary with the Go relay client and the `remote_client` example using main’s `ab-remote` crate (PR #538). It proves TLS WebSocket/smux interoperation, transfer beyond the initial flow-control window, R0 throttling, rotation, revocation and Go↔Rust reopening of the same state file. Never deploy or touch the live relay VM.

## Coordination with ab-relay-control

The forwarder remains the owner of the frozen JSON store and relay/1 protocol. The new `ab-relay-control` worker owns its separate, flag-gated account/invite/device-grant storage and endpoints. No account state or admission bypass is added here. Future integration needs an explicit authenticated policy/admission interface; it must preserve the R0 gate, sticky revocations and durable budget reservations, and must not activate account enrollment during this parity work.

Build host update: prefer `ssh build` with a private `CARGO_TARGET_DIR=~/targets/ab-relay-phase5` once its toolchain is available. Ubuntu verification uses the isolated `~/ab-relay-phase5` directory and `RUSTC_WRAPPER=$HOME/.cargo/bin/sccache` once installed.


## Reproduction

Run on the isolated build host checkout with `PATH=$HOME/go-sdk/bin:$HOME/.cargo/bin:$PATH`, `RUSTC_WRAPPER=$HOME/.cargo/bin/sccache` and a private `CARGO_TARGET_DIR`.

```
cargo build --manifest-path rust/Cargo.toml -p ab-relay --bins --examples
go build -o /tmp/relayrust ./lab/testing/relayrust
/tmp/relayrust "$CARGO_TARGET_DIR/debug/ab-relay" "$CARGO_TARGET_DIR/debug/examples/remote_client"
AB_RELAY_FIXTURE="$CARGO_TARGET_DIR/debug/examples/lab_fixture" AB_LAB_RELAY_STREAM_SECONDS=4 go run ./cmd/bridge-lab run lab/scenarios/core/RELAY-{ACME,ENROLL,LIMITS,OWN-DOMAIN,PASSTHROUGH,ROTATE-GATED,STREAMS,TLS-SPLICE}.yaml --level sim --json
make rust-ci
```

The existing stream-duration override runs the four-second proof across the two-second inactivity budget. The default ten-minute SSE soak was also exercised during development. The adapter scopes child-process cleanup to each scenario's temporary directory. `AB_LAB_GIT_SHA` records the source commit in evidence when an rsynced build directory lacks git metadata.

Control ACME retains Go-compatible SEC1 account and certificate keys, renews within 30 days, accepts newly issued short-lived certificates, and updates the in-memory certificate resolver. The ACME scenario uses Pebble and the injected DNS adapter for both issuance and control-certificate checks. Optional existing iPhone push/APNs support remains separate from tunnel/account authority.
