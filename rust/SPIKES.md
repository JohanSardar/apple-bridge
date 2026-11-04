> Go oracle retired: the Go gateway, its parity oracles and the Go lab were deleted. Go commands, file names and Go-versus-Rust results below record what was verified while Go existed; the stored evidence files are unchanged. The Rust tests and `make rust-ci` are the gate now.

# Phase 1 Rust spikes

Measured 2026-11-02; initially against Go main `10a397d6`, then updated and reverified against Phase 0 main `a02485dd`. No gateway deployment or live-home writes. Database comparisons used online `.backup` snapshots, read-only connections on Ubuntu, and SHA-256 comparisons only. Both local and remote database clones were deleted after comparison. No record or command payloads are retained here.

## S1: Air build budget

**PASS** for the measured skeleton at jobs 1, 2 and 3. All sampled peaks are below 3.5 GB, all leaf checks are below 30 s, and all targets are below 6 GB.

| Jobs | Cold command | Wall seconds | Peak tree RSS MiB | Target MiB | Leaf check seconds |
| --- | --- | ---: | ---: | ---: | ---: |
| 1 | check | 142.563 | 636.2 | 207.2 | 13.088 |
| 1 | build | 113.846 | 701.3 | 434.9 | 15.684 |
| 2 | check | 64.790 | 794.0 | 207.2 | 8.198 |
| 2 | build | 73.323 | 831.4 | 434.9 | 11.501 |
| 3 | check | 48.203 | 942.5 | 207.2 | 8.714 |
| 3 | build | 60.334 | 1094.2 | 434.9 | 8.883 |

Recommend **jobs=2**, one shared target directory, line-table workspace debug information and no dependency debug information. At jobs=2: cold check 64.790 s, cold build 73.323 s, peak build RSS 0.812 GiB, leaf check at most 11.501 s, target 0.425 GiB. Keep Linux builds on Ubuntu and serialize each Air Cargo invocation with `ab-local`, releasing the lock between steps. Jobs=3 passes this skeleton but uses more memory; it does not establish a budget for the future full product. Exact observations are in `spikes/build-budget.json`.


Method: macOS 26.7.1, 8 GiB RAM, 8 logical CPUs, Rust/Cargo 1.99.1. Native arm64 development builds, dependency debug info disabled and workspace line tables enabled. All builds use `ab-local`, `CARGO_BUILD_JOBS=2`, and the shared `~/ab-cargo-target`; explicit `-j 1/2/3` selects the experimental concurrency. `spike-budget` inherits `dev` and isolates cleanup from other workers' profiles. The shared Cargo registry cache is reused, but the measured profile is cleaned before each cold check/build. Registry downloads are not timed separately and may contribute to the first cold observation. The realistic dependency set includes every dependency requested, plus the certificate parser used by S7. These are end-to-end Cargo command timings, not CPU-only compiler timings.

Peak memory is sampled aggregate RSS of Cargo and its descendant processes at 100 ms intervals, excluding unrelated workers. This is a sampled peak, not an OS high-water mark; shared pages may be counted more than once and brief peaks can be missed. Wall time excludes waiting for the build lock and includes sampling overhead. Target disk use is allocated bytes from `du -sk`, for this profile only. A leaf check touches `ab-wire/src/lib.rs` then checks `ab-wire`. Each row has n=1; its p50/p95/max equal the reported observation, with no statistical confidence implied. The initial two rows and the jobs=2 check ran within one lock acquisition; subsequent steps acquire the lock separately so other workers can interleave.

Reproduce from the workspace: `python3 rust/spikes/budget.py`. The controller must run outside `ab-local`; it acquires the lock separately for every clean/check/build/leaf check. Results resume from `/tmp/ab-rust-budget.json`; remove that results file to start a new matrix. The final cleanup removes only `spike-budget` artifacts.

## S2: Helper TCC child attribution

**PASS for the requested launch matrix on this MacBook Air, macOS 26.7.1 (2026-11-02).** The network-free Rust probe is embedded in a newly built lab Helper with bundle ID `com.johansardar.applebridge.services`, signed by certificate SHA-1 `7746ED75674E4CEAF4A091FBCE4C24673EE0ECC4`. The live Helper, gateway and bridge home were not modified. Each real invocation took `~/bin/ab-local`; Cargo used jobs=2 and the shared `~/ab-cargo-target`.

| Launch | Result | Attribution and access evidence |
| --- | --- | --- |
| `open -n -W -g -a <new-app> --args lab-tcc-probe <report>` | PASS | All six reads succeed; the Rust child's responsible PID equals its Helper parent's PID. |
| Same app re-signed with the same certificate, requirements and entitlements, then the same `open` chain | PASS | All six reads succeed again; responsibility still points to the Helper. |
| Byte-identical signed probe copied outside the app and launched as its own GUI launchd agent | DENIED, expected | Parent PID 1; responsible PID equals the probe PID. Database opens fail; WAL and Biome return EPERM. No FDA grant for the standalone probe; the services bundle has FDA `auth_value=2`. The agent is booted out in `finally`. |

The reads are a read-only `SELECT count(*) FROM message` on Messages (168,665 rows), a one-byte open/read of its WAL, existence/readability of one attachment restricted to the Messages Attachments tree, Notes `ZICCLOUDSYNCINGOBJECT` (411 rows), a Biome restricted App.InFocus/local directory listing (2 entries), and Safari `history_items` (2,614 rows). Counts are point-in-time observations; no filenames or content are emitted. WAL readability is tested separately while the normal SQLite count observes its WAL-backed view; this is not a parser test of raw WAL records.

The probe calls the dynamically resolved `responsibility_get_pid_responsible_for_pid` only when present, through lab `ab-sys`. This is diagnostic evidence from a private symbol, not a supported platform guarantee. Exact PIDs, signed artifact hash, designated requirements and FDA rows are retained in `lab/evidence/real/HELPER-TCC-READY.json`. The probe has no network dependencies or network calls and uses a single newline-terminated JSON response over an inherited Unix socketpair. The Helper uses `posix_spawn` without responsibility disclaiming; the entry exists only in LAB builds and returns before normal application initialization.

**Recommendation: use the child for this qualified launch/signing/OS combination.** The conditional staticlib fallback was not needed because child access succeeded. Re-run the scenario after macOS or signing/launch changes; success here does not establish an OS contract. SMAppService registration was not changed or tested. Documents/Desktop/Downloads, Shortcuts, Automation, Accessibility and new permission requests were explicitly excluded by the task's no-prompt rule, overriding the broader plan matrix. No permission dialog was requested.

Re-run: `~/bin/ab-local go run ./cmd/bridge-lab run lab/scenarios/helper/HELPER-TCC-READY.yaml --level real --json`. This scenario builds a new signed destination, checks both Helper launches and all negative controls, and records `impl=rust`; it does not start a gateway or create personal-app fixtures. The isolated lab workspace under `rust/lab/identity-spikes` avoids adding privileged lab dependencies to product crates. Its only unsafe code lives in its `ab-sys` crate.

Verification: the real scenario passed 20 assertions; `make lint`, `go test ./lab/scenarios ./internal/trust`, standalone Cargo Clippy with warnings denied, Cargo formatting and `git diff --check` passed. The dependency tree contains no network crates. These are focused spike gates, not a full-product migration certification.

## S3: gojson

**PASS for the requested synthetic and live-database comparisons. The full plan exit gate remains INCOMPLETE until recorded capture frames are supplied.**

- Generated vectors: **10,042**, **0 mismatches**. Includes 10,000 deterministic exponent/fraction cases, huge and tiny exponents, integers above 2^53, negative zero, duplicate keys, HTML modes, control characters, Unicode separators, lone surrogate keys/values, surrogate pairs, escaped backslashes, and three invalid UTF-8 byte sequences. The invalid UTF-8 cases transport original bytes separately so JSON transport cannot mask replacement differences. Typed command vectors cover year 0001, timezone-to-UTC conversion, nanoseconds/trailing-zero trimming, and maximum u64 versions.
- Live `records.Data`: **195,477 records**, **0 mismatches** against Go `UseNumber` decode and re-marshal.
- Live ledger commands: **11 rows**, **0 mismatches** against the actual `contracts.CommandHash` implementation. Comparison is of recomputed Go and Rust hashes, not a claim about historical stored request hashes.
- `ab-wire::gojson` preserves number tokens using `RawValue`, orders maps, applies Go HTML/no-HTML escaping, replaces unpaired UTF-16 surrogates, formats UTC `GoTime`, and encodes the capitalized command fields in declaration order. It deliberately does not expose a universal struct canonicalizer.
- `tools/lint` rejects the two prohibited serde_json features in manifests. `cargo deny` also rejects their resolved activation, including through transitive dependencies.

The Go oracle that reproduced these synthetic checks (`tools/rust-spike`) is retired with the Go tree; the checks run from `rust/spikes` alone. Optional following database paths invoke the hash-only comparison. Clone with `sqlite3 -readonly`: begin a read transaction, read `sqlite_schema` to establish its snapshot, invoke `.backup` into an owner-only temporary directory, then roll back. The explicit read transaction prevents online-backup restarts under active capture. Never pass a live database to the comparator. No raw capture recording was enabled or read from the gateway.

**Phase 0 golden verification passed after it landed on main `a02485dd`: 22 compatibility comparisons, 0 mismatches, in two Rust integration tests.** These check all four JSON vectors in both escape modes, all four command preimages and hashes, five timestamp normalizations, and the Capture fixture observation Data. This fixture is synthetic; it does not replace a recorded live-capture corpus. Reproduce with `cargo test --manifest-path rust/Cargo.toml -p ab-wire`.

## S5: SQLite parity

**PASS.** Rust `rusqlite=0.41.1`, bundled `libsqlite3-sys=0.39.0`, runtime SQLite **3.54.0**; Go modernc runtime SQLite **3.53.4**.

- Rust compile options: `ENABLE_FTS5=true`, `OMIT_WAL=false`.
- **8 queries, 0 mismatches**, exact ordered row IDs on a five-document synthetic corpus. Both use `unicode61 remove_diacritics 2`; queries cover accents, case, phrases, prefix, Japanese, Boolean OR, and no hits.
- WAL snapshot isolation: a pinned reader retained its count across a committed write from a second connection, then saw the additional row after committing its read transaction. **0 isolation mismatches**.
- Go `state.Open` created the actual schema-v2 state database; Rust reopened it, queried FTS and wrote another document; Go reopened it, observed **6 documents**, and verified `schema_version=2`. **0 reopen mismatches**. This tests state.db, not a migration implementation or live-ledger write compatibility.

Reproduce with `<oracle> sqlite <temporary-path-prefix> <ab-spikes>`. Use a fresh prefix; the probe creates a synthetic home. All synthetic database artifacts from this run are cleaned after verification.

## S7: mTLS

**PASS for the requested mTLS scope.** Rustls **0.23.46**, ring provider, TLS 1.3 only, generated Ed25519 identities, SHA-256 of complete certificate DER, and validity checks. TLS handshake signatures are verified using ring; the custom certificate verifier does not bypass proof of private-key possession.

**10 cases, 0 mismatches**: Go server/Rust client and Rust server/Go client both exchanged application bytes; four wrong-pin cases and four expired-certificate cases were rejected. Go uses the actual `internal/trust` `PinnedListener` and `ClientTLS` implementations. Synthetic identities are generated in a temporary directory and deleted; no live credentials were accessed. The plan's separate smux/relay component is outside this requested mTLS spike and remains unverified.

Reproduced with `python3 rust/spikes/tls_matrix.py <oracle> <ab-spikes>` while the Go `tools/rust-spike` oracle existed; the oracle is retired and the committed results stand as the record.

## S8: legacy Python Keychain item from Rust

**Direct read FAILS non-interactively: OSStatus -25293 (`errSecAuthFailed`).** The test successfully creates and programmatically unlocks a throwaway keychain with a generated 32-byte random password. A Go executable invokes the actual `internal/trust.KeychainStore.Put` path, which runs the frozen python3 ctypes Security.framework script and stores the normal hex encoding. The signed Rust executable uses `security-framework` 3.7.0 with an explicit one-keychain search array and user interaction disabled. Reading the same item returns -25293. The actual Go/Python `Get` path still reads and verifies the original bytes afterwards, proving rollback access and distinguishing the result from missing or corrupt test data.

No login keychain, live bridge credential or existing item was opened by the S8 test. Both Python and Rust disable Keychain interaction before accessing the throwaway store. Any operation requiring UI fails instead of showing a dialog. `SecKeychainDelete` returns 0 in `finally`, and the generated password file is deleted. The test never invokes default-keychain or search-list APIs. These results concern legacy per-item access identity; they do not justify changing live ACLs.

**Migration path:** retain the existing noninteractive Python adapter for old items until an explicit migration is implemented. Use it to read each existing item, write a replacement into the intended store with an explicit ACL for a stable signed Rust executable, then verify Rust and rollback Python reads before retiring the old item. Stop on authentication/interaction errors; do not prompt or silently replace credentials. This spike defines the path but does not implement or authorize a live migration. The live Air's file-store deployment is unaffected.

Re-run both S2 and S8: `~/bin/ab-local python3 rust/lab/identity-spikes/run.py /tmp/ab-rust-identity-results.json`. The Go test writer/reader never prints secret bytes. S8 evidence is retained at `rust/lab/identity-spikes/results.json`; only counts, statuses and process/signing identities are retained. API references: [explicit Keychain search](https://docs.rs/security-framework/3.7.0/security_framework/os/macos/passwords/fn.find_generic_password.html), [disabled interaction](https://docs.rs/security-framework/3.7.0/security_framework/os/macos/keychain/struct.SecKeychain.html#method.disable_user_interaction), [Apple responsibility caveat](https://developer.apple.com/forums/thread/760291).

## Gates and cleanup

Ubuntu: `make lint`, `go test ./...`, Rust build, `cargo clippy --all-targets -- -D warnings`, and `cargo deny check` passed. Cargo-deny reports advisories, licenses, bans and sources OK; duplicate-version warnings are retained rather than suppressed. Rust formatting uses `cargo fmt`. The zero-comment lint has checks for raw/ordinary strings, characters, forbidden comments, the restricted SAFETY exception, doc attributes and the feature bans.

All six Air measurements completed, and the `spike-budget` profile was cleaned (2,045 files / 467.3 MiB removed). Existing shared profiles were preserved. Final golden checks and Rust/Go gates include Phase 0 main `a02485dd` and relay containment main `c76a25d6`. Database clones and temporary synthetic identities are deleted; the isolated Ubuntu workspace, build artifacts and Go toolchain are removed after final verification. The final fresh-clone comparison used the final UTF-8 decoder and observed seven more records than the initial comparison, still with zero mismatches. The plan's 25 GB free-space prerequisite was not met; the task explicitly requested measurements on the available Air. No unrelated worktrees or caches were pruned. These results do not certify the whole migration or authorize a gateway cutover.

Crate/API references: [rusqlite](https://docs.rs/rusqlite/0.41.1/rusqlite/), [rustls certificate verifier](https://docs.rs/rustls/0.23.46/rustls/client/danger/trait.ServerCertVerifier.html), [rmcp features](https://docs.rs/rmcp/3.8.0/rmcp/).

## S4: rmcp parity

**PASS for the frozen transcripts and tested legacy clients: 8/8 exchanges match semantically, 0 breaking differences after adaptation.** Ubuntu x86_64, Rust 1.99.1, Go 1.27.2, rmcp **3.8.0**, Go SDK **1.8.0**. Synthetic loopback servers only; public-host tests select SSE and send synthetic public Host/Origin without contacting a public gateway.

The standalone research crate lives at spikes/s4-s6, outside the product workspace because S6 introduces the dependency advisory below. It uses StreamableHttpService with legacy_session_mode=false, no rmcp attribute macros, and a **147-entry descriptor table** decoded from the frozen tools/list fixture. This proves transport and descriptor serialization, not Rust operation implementations or future ab-catalog generation.

- JSON and SSE: initialize, initialized notification, tools/list and unauthorized tools/call; **4 exchanges per mode, 147/147 descriptors, 0 semantic payload or status mismatches**. Object key order is ignored; array order, schema values, numbers and error code/message are compared. Golden fixtures are unchanged.
- Go and rmcp clients each initialize, list **147 tools** and receive the frozen authorization error code **0** and message: **4 combinations pass**.
- Legacy versions **2024-11-05, 2025-03-26, 2025-06-18, 2025-11-25** are accepted and echoed in both modes: **8 handshakes**. This tests Streamable HTTP negotiation, not the obsolete two-endpoint HTTP+SSE transport.
- Live Go/Rust matrix: **56 comparisons** covering initialize, unknown tool, missing arguments, wrong argument type, unknown method, unknown cancellation ID and malformed JSON, across four versions and two modes. Unknown tools explicitly map to Go error **-32602** and exact message. Unauthenticated invalid arguments follow Go authorization precedence.
- Cancellation: **2/2 Go-client calls** are observed active before cancellation and return to **0 active handlers** within a 2-second deadline. Unknown cancellation notifications return **202** in both implementations. Stateless POSTs have no shared request registry; in-flight cancellation uses HTTP disconnect, matching Go frontdoor request-context cancellation. Cross-POST notification cancellation is not guaranteed.
- Host/Origin: **12/12 checks pass**. Both modes reject evil Host, evil Origin, wrong Origin port and null Origin with **403**; explicitly allowed local and synthetic public origins succeed. Missing Origin remains accepted for SDK clients. Origin allow-list entries include explicit ports.

Every observed difference:

| Difference | Classification/disposition |
| --- | --- |
| JSON property order for initialize and list | Acceptable: objects are unordered; payloads are semantically equal. |
| SSE whitespace/event framing | Acceptable: decoded data events are equal and both SDK clients consume them. Stateless responses have no resumption guarantee. |
| Macro-free handler initially returned authorization for unknown tools | Breaking mapping; fixed by descriptor lookup with Go -32602 and exact message. |
| rmcp malformed application/json returned HTTP 415 versus Go 400 | Breaking HTTP classification; fixed by the application/json status adapter. Other media types retain 415. Remaining diagnostic prose differs, acceptable for malformed requests. |
| Unknown method: Go HTTP 400/plain text versus rmcp HTTP 200/JSON-RPC -32601 | Acceptable unsupported-method error representation; known methods and tools are unaffected. |

Recommendation: **use rmcp**, with descriptor lookup, Go error mapping, malformed-JSON status adaptation, public/local mode selection and explicit allow-lists. A hand-rolled transport is unnecessary for the tested contract. Golden tools/call covers authorization failure only; successful domain calls, application-specific structured errors and production auth middleware remain ab-frontdoor work.

Reproduce on Ubuntu: python3 rust/spikes/s4-s6/run.py. The committed mcp-results.json contains golden, security, legacy-version and rmcp-client observations; the Go matrix logs remaining diagnostic differences. Final focused package wall times are approximately **0.1–0.9 seconds**, n=1 each, smoke timings rather than throughput measurements.

References: [Streamable HTTP](https://modelcontextprotocol.io/specification/2025-03-26/basic/transports), [cancellation](https://modelcontextprotocol.io/specification/2025-03-26/basic/utilities/cancellation), [tool errors](https://modelcontextprotocol.io/specification/2025-03-26/server/tools).

## S6: WebAuthn

**Functional PASS: both directions authenticate, 0 re-enrolments. Dependency acceptance FAIL for webauthn-rs-core 0.6.2.**

Uses the core API directly, never the UUID API. RP localhost, exact origin http://localhost:7780, user handle **18 bytes**, exactly apple-bridge-owner; resident key and UV both **required**. Inputs are the synthetic ceremony corpus and existing Go virtual authenticator. No live passkey, browser, permission prompt or gateway was accessed.

- **Go registration → Rust authentication: 1/1 pass.** Import Go stored ID, COSE public key, counter and flags; recover Rust metadata from archived attestation and compare to independently verified registration. Rust verifies the corpus signature, reports UV=true, counter **1→2**, needs_update=true.
- **Rust registration → Go authentication: 1/1 pass.** Rust creates a fresh challenge/state with the 18-byte handle; the existing Go authenticator signs registration and Rust verifies it. Convert the Rust ES256/P-256 public key to Go COSE-CBOR plus ID/counter/flags. Go FinishDiscoverableLogin authenticates that same credential, checking exact ID and handle.
- Rust also authenticates this Rust-registered key against a fresh Go-authenticator assertion: **1/1 pass**. Required UV is enforced in both implementations: **2/2 freshly signed UV-negative assertions rejected**.
- These ceremonies use the same go-webauthn configuration, handle and discoverable-login API as internal/web/auth.go. Required-resident options are asserted. The virtual authenticator does not prove physical resident-key storage.

| Stored field | Conversion |
| --- | --- |
| ID/key coordinates | Go padded standard base64 versus Rust unpadded base64url; decode to bytes first |
| Public key | Go integer-keyed COSE CBOR versus Rust COSEKey; ES256 EC_EC2 SECP256R1 maps to 1=2, 3=-7, -1=1, -2=x, -3=y |
| Counter | Go authenticator.signCount ↔ Rust counter; persist verified updates |
| UV/backup flags | Preserve user_verified, backup_eligible and backup_state individually |
| Transports | Go golden stores internal; core returned null transports, so preserve Go hint separately |
| Attestation/extensions | Go raw client data/object versus Rust parsed metadata/policy; preserve original Go envelope for rollback |

Recommendation: **core API works without re-enrolment**, with explicit conversion and preservation of the original Go credential envelope byte-for-byte. Do not replace stored Go JSON with Rust Credential JSON. The spike covers **ES256/P-256, none attestation, non-backed-up synthetic credentials**; other algorithms and backup transitions need fixtures before general migration.

The isolated dependency audit found **RUSTSEC-2023-0071** in rsa **0.10.0-rc.19**, through crypto-glue **0.2.2**, with no safe upgrade reported. The spike verifies public-key signatures and performs no RSA private-key operations, but product dependency policy still rejects this dependency. Resolve it before adding WebAuthn to the product workspace. No advisory ignore or policy weakening was added. The initial licence error was missing MIT metadata on this spike, since corrected; upstream MPL-2.0 is already permitted.

### S4/S6 verification

All builds/checks ran on ssh ubuntu; Air use was git/source transfer only. This isolated crate pins both libraries and commits its lockfile. Rust formatting, strict Clippy (pedantic, nursery, unwrap/expect denied), make lint and go test ./... pass. The isolated dependency audit reports the RSA advisory; the product workspace retains its original dependency graph and policy. Focused tests require AB_SPIKE_BINARY, set automatically by run.py; ordinary Go gates skip them without a binary. No product runtime code changed.
