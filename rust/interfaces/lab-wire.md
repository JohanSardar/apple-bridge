# Independent harness wire foundation

`rust/lab/lab-wire` has no product dependency and was implemented from SPEC §7/§12 and the frozen golden inputs. It does not import or copy ab-wire implementations. It owns bounded four-byte big-endian IPC framing, raw JSON envelope fields, IPC HMAC proofs, feed JSON envelopes, feed binary blob chunks, relay tunnel proof messages/headers, and REST/MCP response shapes including SSE data parsing.

The codec gate reads the existing ipc, feed, messages, blob, relay, mcp-json and mcp-sse corpora directly. The other corpora cover product-specific catalog, storage, identity and crypto contracts outside this foundation. Framing preserves supplied JSON payload bytes; it does not claim a replacement for all Go JSON canonicalization paths.

`make rust-lab-independence`, also required by `make rust-ci`, resolves all Cargo features and traverses every dependency reachable from every lab-* or bridge-lab workspace package. A reachable ab-* package fails, including renamed dependencies and paths through third-party shims. The checker also runs positive, negative transitive and cycle probes.
