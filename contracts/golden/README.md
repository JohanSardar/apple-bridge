# Go compatibility freeze

All inputs and cryptographic material here are synthetic and public test vectors. `impl=go` identifies the producer. Files containing JSON strings preserve the exact bytes of each embedded wire message, including key order and escaping. IPC also includes the four-byte big-endian length prefix in hex. These are compatibility decoder tests explicitly requested for Phase 0.

Regenerate from the repository root, review the diff, then run again without the update flag:

```sh
~/bin/ab-local env AB_UPDATE_GOLDEN=1 go test ./contracts/golden ./internal/iphone ./internal/frontdoor ./internal/web ./lab/scenarios -run Golden -count=1
~/bin/ab-local go test ./contracts/golden ./internal/iphone ./internal/frontdoor ./internal/web ./lab/scenarios -run Golden -count=1
```

| Corpus | Producer / verification |
| --- | --- |
| json.json | CommitCapture and same-data version stability, CommandHash preimages and SHA-256, MarshalQueryJSON, RFC3339Nano |
| delegation.json | trust.Delegation raw marshal and Ed25519 signature; preserve raw bytes during verification |
| ipc.json | EncodeFrame / ReadFrame; Open, Ready, Capture, Committed, Ping/Pong, Close |
| feed.json | feed/1 Hello, Welcome, snapshot begin/page/end, batch and partial batch, State snapshot digest |
| relay.json | relay/1 ProofMessage and X-Bridge-Proof handshake, synthetic signature, smux version |
| iphone.json | production sharedCipher and Open, fixed X25519 keys, nonce, Ed25519 seed; salt/info binding and wrong-device rejection |
| mcp-json.json / mcp-sse.json | production frontdoor mcpHandler using current go-sdk, initialize, initialized, full descriptor tools/list, unauthenticated tools/call error |
| passkey.json | existing Go virtual authenticator, resident registration and UV discoverable assertion verified by current WebAuthn implementation |
| oauth.json | production consent/OAuth handler, metadata, owner consent, S256 code exchange and refresh rotation |

OAuth replaces random client IDs, codes and bearer tokens with named synthetic placeholders. The comparison is byte-for-byte after that explicit normalization. Passkey keys, challenges, counter and deterministic RFC6979 signatures are fixed; no native authenticator or permissions are used. The passkey corpus exercises the Go WebAuthn library and virtual authenticator, while focused consent/OAuth sim scenarios verify their HTTP integration. No local golden test claims live Rust parity, cancellation parity or complete protocol coverage; Phase 1 spikes and later scenario gates remain required.
