# ab-session integration contract

Owner worktree: /Users/johansardar/.t3/worktrees/apple-bridge/t3-6e4deed1
Branch: t3/ab-session-native-proxy. Canonical protocol: design/spec/native-1.md.

## Launch / default proxy (Phase 6 step 1)

Helper opens the gateway Unix socket WITHOUT authenticating, creates a local socketpair,
and posix_spawns its bundled ab-session child without disclaiming responsibility (S2).
Child arguments: `--gateway-fd 198 --helper-fd 199`. Both descriptors are connected Unix
streams, distinct, >=3. Child owns inherited descriptors. Parent closes child ends and its
copy of gateway fd. Existing PeerWire uses parent socketpair end in place of connecting;
its existing Challenge/Authenticate handshake and all other IPC logic remain unchanged.
Proxy relays raw bytes including authentication; it does not parse or re-encode frames.
EOF / errors terminate the connection; parent reaps child before reconnect. No daemon,
network listener, secret parsing, capture assignment, or guard in the child proxy path.
Default remains existing direct Swift connection; opt in with AB_SESSION_PROXY=1 and
AB_SESSION_BINARY (lab override; production bundled executable).

## Native endpoint (separate socketpair, inactive in proxy mode)

AB_NATIVE_FD supplies an inherited connected stream. Native/1 MUST NOT share the IPC
proxy stream. Rust client sends {"t":"Hello","protocol":"native/1","version":1,"owner_fence":1};
Swift replies {"t":"Welcome","protocol":"native/1","version":1}.
Frames: u32 big-endian byte length + UTF-8 JSON object, 1..262144 bytes. Unknown t/op,
wrong version or missing required field rejected. Requests correlate by nonempty id.

Fact request: {t:Facts,id,op,args,snapshot:null,cursor:null,limit:100}; subsequent pages
repeat op,args,snapshot and cursor. FactPage: {t:FactPage,id,op,snapshot,items:[],cursor:null,
complete:true,scope,generation}. Snapshot token opaque and pinned; complete true ONLY
on successful terminal exhaustive page, no missing rows, stable scope/generation.
Interrupted/truncated/expired scans never imply absence. Error: {t:Error,id,code}.
Codes: invalid,unsupported,unavailable,permission_missing,expired,conflict,corrupt.
Closed fact ops: agenda.calendars, agenda.events, agenda.reminder_lists, agenda.reminders,
contacts.containers, contacts.groups, contacts.contacts, contacts.history, ui.target,
ui.actions, focus.status, location.status, mac.state, permissions.status. No generic SQL/script selector.

Effect request: {t:Effect,id,op,args,permit:{command_id,owner_fence,ticket,request_hash,
operation,expires_at,nonce}}. EffectResult: {t:EffectResult,id,evidence:{...}}.
Permits bind op, canonical request hash, fence, ticket, expiry and one-shot nonce.
Swift rejects missing permit, wrong op/fence/hash, expired or reused nonce before effect.
Rust alone mints permit after fsync DispatchStarted, feature `rust-effects` OFF by default.
No automatic fallback between Swift and Rust command authority. Queries which trigger
framework/network work (Vision, Speech, Maps) are effects too. Permission facts never prompt.
File refs: {path,size,sha256,mime}; confined to session-owned 0700 exchange root, regular
0600 files, no symlink traversal; receiver verifies bytes/hash and acknowledges release.

## Reader ownership API

Session owns selection; readers emit observations only (no gateway handle). Per source:
Swift (default), Shadow (Swift authoritative, Rust diff only), Rust (future routed mode).
Proxy rejects any configured authoritative Rust producer; it cannot safely filter IPC.
One external session. In Swift/Shadow mode Swift assigns capture IDs, spends credits,
receives Committed and persists checkpoints. In routed Rust mode ab-session does all four;
ab-native only reads/diffs. Switch only disconnected after all captures committed;
reopen with committed checkpoint, exactly one selected producer. No hot takeover.
Shadow compares key+canonical data+scope+generation+completeness; never sends captures,
spends credits, acknowledges or persists an authoritative checkpoint. Partial scans cannot
certify equality or deletion. Native/1 facts are NOT ipc/1 Captures.

Native request_hash: lowercase SHA256 hex of canonical args JSON (recursively sorted keys, compact UTF-8, unescaped slash, no non-finite numbers). Separate from frozen ipc/1 request_hash. Hello includes positive owner_fence; permit must match it. expires_at is Unix seconds (integer).

Swift integration agreement: Helper reserves separate native socketpair child fd 200 via AB_NATIVE_FD; raw IPC proxy does not read it or send Hello. Native server rejects effects conflict while Swift IPC owns authority. Source adapters initially support full bounded scope args={} only; unsupported filters return unsupported. contacts.history accepts history_token. Scope may be structured {collections,history_token}; pages bind exact scope equality. Release uses args.file_ref and replies Released with id.

Binary selection configuration: AB_SESSION_PRODUCERS JSON object maps source names to
"Swift", "Shadow", "Rust". Default Swift. Rust fails closed in this proxy build.
Reader integration: session-owned adapters wrap ab-native readers and implement ownership::ShadowReader::snapshot(SourceId) -> ObservationSet (ab-native must not depend on ab-session);
call shadow_once with Swift fact adapter and ab-native reader. API accepts no writer or
checkpoint sink and returns Equal/Incomplete/Different. Records are canonical JSON bytes
keyed by native key; no duplicate keys. ab-native adapters remain worker-owned.
