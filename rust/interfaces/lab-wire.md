> Go oracle retired: the Go gateway, its parity oracles and the Go lab were deleted. Go commands, file names and Go-versus-Rust results below record what was verified while Go existed; the stored evidence files are unchanged. The Rust tests and `make rust-ci` are the gate now.

# Independent harness wire foundation

`rust/lab/lab-wire` has no product dependency and was implemented from SPEC §7/§12 and the frozen golden inputs. It does not import or copy ab-wire implementations. It also provides an independent JSON tree preserving number tokens, sorted object keys and Go HTML escaping, checked against record/query bytes and command preimages/hashes. It owns bounded four-byte big-endian IPC framing, raw JSON envelope fields, IPC HMAC proofs, feed JSON envelopes, feed binary blob chunks, relay tunnel proof messages/headers, and REST/MCP response shapes including SSE data parsing.

The codec gate reads the existing json, ipc, feed, messages, blob, relay, mcp-json and mcp-sse corpora directly. The other corpora cover product-specific catalog, storage, identity and crypto contracts outside this foundation. Framing preserves supplied JSON payload bytes; the JSON tree separately preserves integers beyond u64, exponent spellings and negative zero without enabling serde_json arbitrary_precision.

`make rust-lab-independence`, also required by `make rust-ci`, resolves all Cargo features and traverses every dependency reachable from every lab-* or bridge-lab workspace package. A reachable ab-* package fails, including renamed dependencies and paths through third-party shims. The checker also runs positive, negative transitive and cycle probes.
