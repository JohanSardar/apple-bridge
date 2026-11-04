# OAuth review: what stays hand-written and why

Reviewed 2026-11-04: `rust/crates/ab-trust/src/oauth.rs`, `rust/crates/ab-trust/src/passkeys.rs` and the OAuth routes in `rust/crates/ab-frontdoor`. Requirements are from [Claude's connector authentication docs](https://claude.com/docs/connectors/building/authentication) and [ChatGPT's MCP auth docs](https://developers.openai.com/apps-sdk/build/auth).

## The decision

The gateway's authorization server stays ours, in Rust. It runs on the owner's Mac, and the owner's Mac is what approves. Nothing maintained covers that shape.

| Candidate | Result | Why |
|---|---|---|
| `oxide-auth` (Rust server framework) | Rejected | Last release June 2024. No RFC 8707 resource binding or RFC 9207 `iss`, which ChatGPT needs for its stable redirect, and no client ID metadata documents. We would write the hard parts anyway, around an unmaintained core. |
| Better Auth `oauthProvider` + `mcp` | Rejected for the gateway | It covers the protocol, but needs a Node runtime and database on every Mac, and moves approval logic out of Rust. It is used for accounts, where it fits. |
| `rmcp` | Used for MCP | Its auth support is client side. It has no authorization server. |
| `oauth2`, `openidconnect` (Rust clients) | `oauth2` used for PKCE | Client libraries. They contain no server logic to adopt beyond primitives. |

## Replaced with libraries

| Was | Now |
|---|---|
| PKCE challenge built by hand | `oauth2::PkceCodeChallenge::from_code_verifier_sha256` |
| Token comparison through a hand-rolled HMAC construction | `subtle::ConstantTimeEq` over SHA-256 digests |
| Cookie parsing and `Set-Cookie` strings built by hand | `cookie` crate |
| Passkey verification | Already `webauthn-rs-core`, kept. No hand-written WebAuthn remains in the gateway. |

The relay control plane had hand-written WebAuthn and GitHub OAuth. Both are replaced by Better Auth. See `services/accounts`.

## Kept, and what holds it to account

These have no maintained library, because they are this server's policy rather than protocol plumbing: the code and refresh token store, redirect matching, client ID metadata document fetching with its SSRF checks, the authorization endpoint's error routing, and metadata documents.

Two kinds of test hold them to the specs:

- `rust/crates/ab-frontdoor/tests/oauth_compliance.rs` runs the requirements above through the real HTTP routes, using the metadata the real clients publish.
- `lab/compliance/` runs clients we did not write: `oauth4webapi`, a strict client library, and the MCP SDK's own `auth()`. They reject non-conforming metadata and responses.

## Found and fixed in this review

| Problem | Effect | Fix |
|---|---|---|
| Each public hostname had its own in-memory approval queue, and the owner's socket read only the local one | A connector coming through the relay could never be approved. Sims passed because they run on localhost. | One queue per gateway |
| The page that asks the Mac or iPhone had been deleted with the old web UI | The page offered only a passkey | New approval page |
| The gateway never handed connector requests to the iPhone, and nothing opened the Mac window | No approval surface appeared | Wired both |
| Loopback redirects matched exactly | Claude Code's published metadata lists `http://localhost/callback` and calls back on an ephemeral port, so it would fail. RFC 8252 section 7.3. | Port-agnostic match, loopback only |
| `offline_access` was not advertised, and a token request naming it was refused | Claude only asks for refresh tokens when it is advertised | Advertised and accepted |
| Registration refused a missing `client_name` or a requested auth method | RFC 7591 lets the server replace these | Replaced in the response |
| Registered clients were capped at 1000, never removed | Anyone could fill the cap and block new connections | Never-used registrations older than a day are evicted at the cap |
| A repeated refresh token always revoked the agent | A client retrying over a dropped connection would lose its connection and need re-approval | A repeat within 60 seconds rotates. Later reuse still revokes. |
| Authorization errors returned JSON to the browser | Clients never learned why | Redirect with `error`, `state` and `iss` once the client and redirect are verified; a plain page before that |
| `WWW-Authenticate` pointed at the root metadata path | RFC 9728 derives `/.well-known/oauth-protected-resource/mcp` for this resource | Points at the path-suffixed document. Both are served. |

## Accepted limits

- A repeat within the retry window issues a new token pair, so a stolen refresh token used within 60 seconds of rotation is not detected at that moment. Its use after that is.
- All clients are public. A client ID metadata document declaring `private_key_jwt` still authenticates as `none`, which is the only method our metadata offers. PKCE and the document's redirect list bind the client.
- Tokens are opaque and checked against the gateway's store on each call, so `aud` is the store's `resource` field rather than a signed claim.
