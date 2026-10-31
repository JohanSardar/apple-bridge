package iphone

import (
	"bytes"
	"context"
	"crypto/ecdh"
	"crypto/ed25519"
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"encoding/json"
	"net/http/httptest"
	"testing"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func TestConnectorApprovalDeliveryAndAuthenticatedDecision(t *testing.T) {
	t.Setenv("APPLE_BRIDGE_SECRET_STORE", "file")
	authority, err := trust.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	store, err := state.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	defer store.Close()
	if err := store.InitCommands(context.Background()); err != nil {
		t.Fatal(err)
	}
	raw, err := authority.PairIPhoneCode("https://bridge.example", false, time.Minute)
	if err != nil {
		t.Fatal(err)
	}
	code, err := trust.DecodePairCode(raw)
	if err != nil {
		t.Fatal(err)
	}
	public, private, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	encryption, err := ecdh.X25519().GenerateKey(rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	message := trust.IPhonePairMessage(code.ID, public, encryption.PublicKey().Bytes())
	key := sha256.Sum256(code.Secret)
	mac := hmac.New(sha256.New, key[:])
	mac.Write(message)
	phone, err := authority.PairIPhone(trust.IPhonePairRequest{ID: code.ID, SigningKey: public, EncryptionKey: encryption.PublicKey().Bytes(), Proof: mac.Sum(nil), Signature: ed25519.Sign(private, message)})
	if err != nil {
		t.Fatal(err)
	}
	principal, err := authority.Principal(phone.ID)
	if err != nil {
		t.Fatal(err)
	}
	delivery := NewDelivery(store, authority)
	expiry := time.Now().Add(5 * time.Minute)
	delivery.ConnectorPending = func() []ConnectorApproval {
		return []ConnectorApproval{{ID: "connector:test", Code: "1234", Name: "[ab-test] connector", Scopes: []contracts.Grant{"messages:read"}, Expires: expiry}}
	}
	packets, err := delivery.packets(context.Background(), principal, true)
	if err != nil || len(packets) != 1 {
		t.Fatalf("packets=%d err=%v", len(packets), err)
	}
	plain, err := Open(encryption, phone.ID, packets[0], authority.IPhoneGatewayKey())
	if err != nil {
		t.Fatal(err)
	}
	var job Job
	if err := json.Unmarshal(plain, &job); err != nil {
		t.Fatal(err)
	}
	var args map[string]string
	json.Unmarshal(job.Command.Args, &args)
	if args["device_id"] != phone.ID || job.Command.Op != "bridge.approval" || job.Command.ID != "connector:test" || job.Hash != "1234" || job.Expires != expiry.Unix() || !bytes.Contains(job.Command.Args, []byte("Code 1234")) || !bytes.Contains(job.Command.Args, []byte("messages:read")) {
		t.Fatal("connector prompt is missing its device, code, scopes or expiry")
	}
	called := false
	delivery.Decide = func(context.Context, string, string, bool) (contracts.CommandRecord, error) {
		t.Fatal("connector reached command approval")
		return contracts.CommandRecord{}, nil
	}
	delivery.ConnectorDecide = func(id, matching string, approve bool) error {
		called = true
		if id != "connector:test" || matching != "1234" || !approve {
			t.Fatal("decision lost request binding")
		}
		return nil
	}
	payload, _ := json.Marshal(map[string]any{"command_id": "connector:test", "request_hash": "1234", "approve": true})
	signed := trust.IPhoneRequest{DeviceID: phone.ID, At: time.Now().Unix(), Payload: payload}
	signed.Signature = ed25519.Sign(private, trust.IPhoneMessage("/v1/iphone/approval", signed.At, payload))
	send := func(request trust.IPhoneRequest) int {
		body, _ := json.Marshal(request)
		response := httptest.NewRecorder()
		delivery.Handler().ServeHTTP(response, httptest.NewRequest("POST", "https://bridge.example/v1/iphone/approval", bytes.NewReader(body)))
		return response.Code
	}
	forged := signed
	forged.Payload = []byte(`{"command_id":"connector:other","request_hash":"1234","approve":true}`)
	if send(forged) != 401 || called {
		t.Fatal("forged approval accepted")
	}
	if send(signed) != 200 || !called {
		t.Fatal("signed connector decision refused")
	}
}
