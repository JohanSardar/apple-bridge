# Hosted domain migration qualification, 2026-11-08

The reviewed relay migration is deployed. Legacy and canonical control/sign-in
origins answer over trusted TLS, a retained Mac identity uses the new control
endpoint, and a separate fixture Mac connects through a new customer hostname.
Real Claude OAuth and a read-only MCP query passed. Physical iPhone application
continuity remains unverified.

This report supports review of PR #948. It contains counts, statuses and artifact
fingerprints; private operational backups, device identifiers, session material,
raw UI state and host configuration remain outside the repository.

## Source and artifact mapping

The full implementation received independent security/data-loss approval at
`93da594894d670a3a8b65bfb00020540b05778d1`. The propagation-cache follow-up received
focused independent security approval at
`f3538b3dc02ba6fdda501268c4603d73c9460c06`, with no findings. The latter is the
source head used for the deployed Rust artifacts.

| Deployed artifact | SHA-256 |
| --- | --- |
| Relay | `840a43ed46a9373e82fce558dc0d556841abf7037868f6e9f0b72c1a5e646bc7` |
| Control | `9c1ca53927bc706339deea1753c8b111203794247d21f6aa621e30a520b14a66` |
| Both accounts services | `51087a2e8bca99264b3e96ea50a497681e8b5610bbd45b4c7273a5119e2d6d26` |
| Provisioned accounts dependency carrier | `e59192fd5bcaaa712645376c5008d5aa32eb8305a4b2df48f850ac486ed8930f` |
| Public installer script, canonical and legacy URLs | `0aa7897b64f822d13dd0484a78bc627ecb86b47c0c038c8f553047899a51b6fe` |

Rust builds ran on the authorized Linux build VM with a private checkout and
Cargo target directory. Hashes were captured after the final build completed,
then matched against private staging and deployed files. Both accounts service
ExecStart targets use the same verified accounts artifact. The carrier was
independently compared with the existing production executable; its public
Better Auth 1.7.17 dependencies were reused without npm.

## Verification

At the deployed source head, `make lint`, `make features`,
`make rust-lab-independence`, 58 focused relay/control tests, Clippy with warnings
denied, and both binary builds passed. The unchanged accounts source passed 17
fixtures, including two actual service processes starting concurrently against a
fresh database, concurrent session/passkey writes, integrity checks, transaction
commit/rollback and bounded lock contention. Golden contracts passed at the
prior implementation head; the cache follow-up changes no wire contract.

The propagation regression uses two independent UDP peers with 3600-second
negative/stale answers. Restoring the client cache fails the regression with
exit 101; the uncached implementation passes with exit 0. One correct resolver
and one wrong resolver do not satisfy the check. Both fixed public resolver
checks, exact TXT matching and the existing ACME deadline are preserved.

## Observed live results

- All three new control, sign-in and apex certificates issued through the real
  DNS-01 path. Hostname, expiry and trusted TLS checks passed before cutover.
  The apex used the explicitly delegated ACME CNAME target.
- Both accounts services, control and relay were stopped for the coordinated
  upgrade. Private consistent database backups and original executables/config
  were retained. Both accounts health checks and executable digests passed
  before both control health checks and relay reopening.
- Legacy and canonical relay `/info` returned 200. Both sign-in pages returned
  200. The existing tagged account signed in at both origins with 200; each
  returned registration options for its own RP ID. Session cookies were Secure,
  HttpOnly and host-only with the `__Host-` name.
- Actual Chrome probes ran from both legacy and new customer origins. A control
  cookie write succeeded; parent-domain writes for the session/invite cookie
  names were rejected; probe cookies were removed. Cross-origin responses were
  inaccessible to browser JavaScript. Authenticated HTTP requests carrying each
  customer Origin received 403 from the corresponding sign-in control service.
- The retained fixture Mac changed only its control endpoint while preserving
  its device key, credential and customer URL. Its legacy customer OAuth
  discovery returned 200 and unauthenticated MCP returned 401. `/health` is not
  a gateway route (404); this was not treated as a service failure.
- A separate fixture identity enrolled through the canonical hosted device-link
  flow (grant view and confirmation 200), received an `.applebridge.eilmer.si`
  hostname and served trusted-TLS OAuth discovery with 200.
- Real signed-in Claude connected to that new customer host through OAuth and
  owner-local matching-code approval. Its UI showed the Records query tool on
  the tagged connector and reported 9 `reminders.list` records in one page.
  The requested operation was read-only.
- A synthetic phone paired once through the real relay (200), rejected replay
  (401), and returned the same signed status (200) before and after the Mac's
  control-endpoint migration. The phone principal and Mac identity remained
  intact without re-pairing. Only that created phone fixture was revoked after
  the test.
- Independent website QA verified root/product/slash HTML 200, CSS type,
  plaintext installer 200, unknown-path 404, all four published file bytes,
  legacy installer parity, and desktop/narrow browser layouts.

## Limits

No authorized paired physical-phone fixture was available. The signed fixture
qualifies protocol and retained-pairing behavior over the real relay; it does
not qualify physical iPhone app execution, push delivery or saved app state.
Neither existing empty pairing stores nor historical phone reports are claimed
as current physical-device proof.

The future scheduled certificate-renewal cycle was not waited for. Issuance,
certificate loading and the actual dual-resolver propagation path were observed.
The first preflight timed out on a stale cached DNS answer without changing the
live listener; the reviewed cache fix then issued the missing certificates in
one fresh bounded preflight. No unbounded issuance restart loop was used.

The hosted origins intentionally share an account/session audience and secret.
Browser host isolation is verified; server-side token audience separation is
not promised. Existing customer URLs remain available; changing an existing
Mac to a custom customer domain is a separate explicit action because its
current single-host certificate path can replace the original hostname.
