package golden_test

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"crypto/hmac"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"testing"
	"time"

	"github.com/JohanSardar/apple-bridge/contracts/golden"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/relay"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func encode(t *testing.T, v any) []byte {
	t.Helper()
	b, e := json.MarshalIndent(v, "", "  ")
	if e != nil {
		t.Fatal(e)
	}
	return append(b, '\n')
}
func TestJSONGolden(t *testing.T) {
	ctx := context.Background()
	store, e := state.Open(t.TempDir())
	if e != nil {
		t.Fatal(e)
	}
	defer store.Close()
	at := time.Date(2026, 1, 2, 3, 4, 5, 123456789, time.UTC)
	inputs := []string{`{"z":9007199254740993,"a":1.0,"b":1e0,"c":-0,"d":-0.0}`, `{"text":"雪 😀 <>& \u2028\u2029","null":null,"array":[]}`, `{"n":123456789012345678901234567890,"e":1E+09,"small":1e-09}`, `{"duplicate":1,"duplicate":2}`}
	rows := []any{}
	predecessor := ""
	for i, input := range inputs {
		d := json.NewDecoder(bytes.NewBufferString(input))
		d.UseNumber()
		var v any
		if e = d.Decode(&v); e != nil {
			t.Fatal(e)
		}
		canonical, e := json.Marshal(v)
		if e != nil {
			t.Fatal(e)
		}
		query, e := contracts.MarshalQueryJSON(v)
		if e != nil {
			t.Fatal(e)
		}
		cmd := contracts.Command{Op: "fixture.op", Args: json.RawMessage(input), ExpectedVersion: 7, ExpiresAt: at, AdmissionEra: "era-fixture"}
		hash, e := contracts.CommandHash(cmd)
		if e != nil {
			t.Fatal(e)
		}
		preimage, e := json.Marshal(struct {
			Op      string
			Args    any
			Version uint64
			Expires time.Time
			Era     string
		}{cmd.Op, v, 7, at, cmd.AdmissionEra})
		if e != nil {
			t.Fatal(e)
		}
		sum := sha256.Sum256(preimage)
		if hash != hex.EncodeToString(sum[:]) {
			t.Fatal("hash preimage mismatch")
		}
		capture := contracts.Capture{Source: contracts.Contacts, Generation: "fixture", Checkpoint: string(rune('a' + i)), Observations: []contracts.Observation{{Kind: "upsert", Collection: "contacts.contact", NativeKey: "fixture", ObservedAt: at, Data: json.RawMessage(input)}}}
		capture.Predecessor = predecessor
		if _, e = store.CommitCapture(ctx, capture); e != nil {
			t.Fatal(e)
		}
		result, e := store.Read(ctx, trust.AllVisibility(), contracts.ReadQuery{Collection: "contacts.contact"})
		if e != nil || len(result.Records) != 1 {
			t.Fatalf("read %v", e)
		}
		record := result.Records[0]
		if !bytes.Equal(record.Data, canonical) {
			t.Fatalf("CommitCapture bytes %s != %s", record.Data, canonical)
		}
		capture.Predecessor = capture.Checkpoint
		capture.Checkpoint += "-retry"
		if _, e = store.CommitCapture(ctx, capture); e != nil {
			t.Fatal(e)
		}
		again, e := store.Read(ctx, trust.AllVisibility(), contracts.ReadQuery{Collection: "contacts.contact"})
		if e != nil || again.Records[0].Version != record.Version {
			t.Fatal("identical bytes bumped version")
		}
		rows = append(rows, map[string]any{"input": input, "record_data": string(canonical), "query": string(query), "command_preimage": string(preimage), "command_hash": hash})
		predecessor = capture.Checkpoint
	}
	times := []string{}
	for _, n := range []int{0, 1, 1000, 123000000, 123456789} {
		times = append(times, time.Date(2026, 1, 2, 3, 4, 5, n, time.FixedZone("offset", 19800)).Format(time.RFC3339Nano))
	}
	golden.Check(t, "json.json", encode(t, map[string]any{"impl": "go", "vectors": rows, "rfc3339nano": times}))
}

func TestWireGolden(t *testing.T) {
	at := time.Date(2026, 1, 2, 3, 4, 5, 123456789, time.UTC)
	capture := contracts.Capture{Source: contracts.Contacts, Generation: "fixture", Checkpoint: "cp-1", Observations: []contracts.Observation{{Kind: "upsert", Collection: "contacts.contact", NativeKey: "fixture", ObservedAt: at, Data: json.RawMessage(`{"name":"<>& 雪"}`)}}}
	frames := []contracts.IPCFrame{{T: "Open", Protocol: contracts.IPCProtocol, OwnerFence: 7, ByteCredit: contracts.ByteCredit}, {T: "Ready", Protocol: contracts.IPCProtocol, HelperBuild: "synthetic"}, {T: "Capture", OwnerFence: 7, Capture: &capture}, {T: "Committed", Source: contracts.Contacts, Checkpoint: "cp-1", Credit: 512}, {T: "Ping"}, {T: "Pong"}, {T: "Close", Reason: "gateway_stopped"}}
	out := []any{}
	for _, f := range frames {
		b, e := contracts.EncodeFrame(f)
		if e != nil {
			t.Fatal(e)
		}
		var decoded contracts.IPCFrame
		if _, e = contracts.ReadFrame(bytes.NewReader(b), &decoded); e != nil {
			t.Fatal(e)
		}
		out = append(out, map[string]any{"json": string(b[4:]), "framed_hex": hex.EncodeToString(b)})
	}
	mac := hmac.New(sha256.New, []byte("synthetic-ipc-secret"))
	mac.Write([]byte("apple-bridge-ipc/1\x00helper\x00nonce-a\x00nonce-b"))
	golden.Check(t, "ipc.json", encode(t, map[string]any{"impl": "go", "frames": out, "hmac_input": "apple-bridge-ipc/1\x00helper\x00nonce-a\x00nonce-b", "hmac_sha256": hex.EncodeToString(mac.Sum(nil))}))
	r := contracts.Record{Collection: "contacts.contact", ID: "con_fixture", Version: 1, Source: contracts.Contacts, ObservedAt: at, Data: json.RawMessage(`{"name":"fixture"}`)}
	digest := state.Digest([]contracts.Record{r})
	feed := []contracts.FeedFrame{{T: "Hello", Protocol: contracts.FeedProtocol, ReplicaID: "replica-fixture", SchemaOffer: []int{2}, Credits: contracts.ByteCredit}, {T: "Welcome", Protocol: contracts.FeedProtocol, GatewayID: "gateway-fixture", Epoch: "epoch-fixture", Schema: 2, SubscriptionRevision: 1}, {T: "SnapshotBegin", ID: "snapshot-fixture", S: 1, Count: 1, Digest: digest, ExpiresAt: at}, {T: "SnapshotPage", ID: "snapshot-fixture", Records: []contracts.Record{r}}, {T: "SnapshotEnd", ID: "snapshot-fixture", Digest: digest}, {T: "Batch", Batch: &contracts.Batch{Epoch: "epoch-fixture", SubscriptionRevision: 1, After: 1, Through: 2, Changes: []contracts.Change{{Sequence: 2, Op: "upsert", Record: r}}}}, {T: "Batch", Batch: &contracts.Batch{Partial: true, Epoch: "epoch-fixture", SubscriptionRevision: 1, After: 2, Through: 2, Changes: []contracts.Change{}}}}
	out = nil
	for _, f := range feed {
		b, e := contracts.MarshalQueryJSON(f)
		if e != nil {
			t.Fatal(e)
		}
		out = append(out, string(b))
	}
	golden.Check(t, "feed.json", encode(t, map[string]any{"impl": "go", "frames": out}))
	key := ed25519.NewKeyFromSeed(bytes.Repeat([]byte{42}, 32))
	proof := relay.Proof{Hostname: "fixture.example.com", Credential: "synthetic-credential", Nonce: "synthetic-nonce"}
	message := relay.ProofMessage(proof, "tunnel")
	proof.Signature = base64.RawURLEncoding.EncodeToString(ed25519.Sign(key, message))
	b, e := json.Marshal(proof)
	if e != nil {
		t.Fatal(e)
	}
	golden.Check(t, "relay.json", encode(t, map[string]any{"impl": "go", "challenge_response": `{"nonce":"synthetic-nonce"}`, "proof_message": string(message), "proof_json": string(b), "X-Bridge-Proof": base64.RawURLEncoding.EncodeToString(b), "public_key": base64.RawURLEncoding.EncodeToString(key.Public().(ed25519.PublicKey)), "request": "GET /tunnel", "smux_version": 2}))
	d := trust.Delegation{ReplicaID: "replica-fixture", LocalPrincipal: "token:fixture", LocalGrants: []contracts.Grant{"contacts:read"}, CommandID: "cmd-fixture", RequestHash: "hash-fixture", IAT: 1767323045}
	b, e = json.Marshal(d)
	if e != nil {
		t.Fatal(e)
	}
	sig := ed25519.Sign(key, b)
	golden.Check(t, "delegation.json", encode(t, map[string]any{"impl": "go", "raw": string(b), "header": base64.RawURLEncoding.EncodeToString(b) + "." + base64.RawURLEncoding.EncodeToString(sig), "public_key": base64.RawURLEncoding.EncodeToString(key.Public().(ed25519.PublicKey))}))
}

func TestBlobChunkGolden(t *testing.T) {
	rows := []any{}
	for _, offset := range []int64{0, 1, 0x0102030405060708} {
		digest := "2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a"
		b, err := contracts.EncodeBlobChunk(digest, offset, []byte("fixture"))
		if err != nil {
			t.Fatal(err)
		}
		rows = append(rows, map[string]any{"digest": digest, "offset": offset, "payload": "fixture", "hex": hex.EncodeToString(b)})
	}
	golden.Check(t, "blob.json", encode(t, map[string]any{"impl": "go", "chunks": rows}))
}

func TestAllMessagesGolden(t *testing.T) {
	ipc := []any{}
	for _, kind := range []string{"Challenge", "Authenticate", "Authenticated", "Open", "Ready", "Committed", "SourceState", "Ping", "Pong", "Close", "Reset", "Prepare", "Prepared", "Dispatch", "Rejected", "Outcome", "Resolve", "BlobUnavailable", "BlobRead", "BlobChunk", "Query", "QueryResult", "QueryError", "QueryCancel"} {
		frame := contracts.IPCFrame{T: kind, ID: "qry_fixture", Op: "fixture.op", Args: json.RawMessage(`{"text":"<>&"}`), ResultJSON: json.RawMessage(`{"ok":true}`), Code: "invalid", CommandID: "cmd_fixture", OwnerFence: 7, Handle: "fixture", Bytes: []byte("fixture")}
		b, err := contracts.EncodeFrame(frame)
		if err != nil {
			t.Fatal(err)
		}
		ipc = append(ipc, map[string]any{"json": string(b[4:]), "framed_hex": hex.EncodeToString(b)})
	}
	feed := []any{}
	for _, kind := range []string{"Hello", "Welcome", "SnapshotBegin", "SnapshotPage", "SnapshotEnd", "Ack", "Reset", "Heartbeat", "BlobWant", "BlobDone", "CommandRequest", "CommandResponse", "Clock"} {
		frame := contracts.FeedFrame{T: kind, Protocol: contracts.FeedProtocol, ID: "fixture", Request: json.RawMessage(`{"op":"fixture.op","args":{"text":"<>&"}}`)}
		b, err := contracts.MarshalQueryJSON(frame)
		if err != nil {
			t.Fatal(err)
		}
		feed = append(feed, string(b))
	}
	golden.Check(t, "messages.json", encode(t, map[string]any{"impl": "go", "ipc": ipc, "feed": feed}))
}
