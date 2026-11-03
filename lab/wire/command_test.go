package wire

import (
	"bytes"
	"crypto/ed25519"
	"encoding/base64"
	"encoding/json"
	"os"
	"strings"
	"testing"
	"time"
)

func TestCommandHashGolden(t *testing.T) {
	raw, err := os.ReadFile("../../contracts/golden/json.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		Vectors []struct {
			Input string `json:"input"`
			Hash  string `json:"command_hash"`
		} `json:"vectors"`
	}
	if err = json.Unmarshal(raw, &corpus); err != nil {
		t.Fatal(err)
	}
	expiry, err := time.Parse(time.RFC3339Nano, "2026-01-02T03:04:05.123456789Z")
	if err != nil {
		t.Fatal(err)
	}
	for i, v := range corpus.Vectors {
		got, err := CommandHash("fixture.op", json.RawMessage(v.Input), 7, expiry, "era-fixture")
		if err != nil || got != v.Hash {
			t.Fatalf("hash vector %d: %v %s", i, err, got)
		}
	}
}

func TestDelegationGolden(t *testing.T) {
	raw, err := os.ReadFile("../../contracts/golden/delegation.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		Header string `json:"header"`
		Public string `json:"public_key"`
		Raw    string `json:"raw"`
	}
	if err = json.Unmarshal(raw, &corpus); err != nil {
		t.Fatal(err)
	}
	var value Delegation
	if err = json.Unmarshal([]byte(corpus.Raw), &value); err != nil {
		t.Fatal(err)
	}
	encoded, err := json.Marshal(value)
	if err != nil || string(encoded) != corpus.Raw {
		t.Fatal("delegation payload differs from golden")
	}
	parts := strings.Split(corpus.Header, ".")
	if len(parts) != 2 {
		t.Fatal("golden delegation envelope")
	}
	public, err := base64.RawURLEncoding.DecodeString(corpus.Public)
	if err != nil {
		t.Fatal(err)
	}
	signature, err := base64.RawURLEncoding.DecodeString(parts[1])
	if err != nil || !ed25519.Verify(public, encoded, signature) {
		t.Fatal("golden delegation signature")
	}
	key := ed25519.NewKeyFromSeed(bytes.Repeat([]byte{7}, ed25519.SeedSize))
	header, err := SignDelegation(key, value)
	if err != nil {
		t.Fatal(err)
	}
	parts = strings.Split(header, ".")
	payload, err := base64.RawURLEncoding.DecodeString(parts[0])
	if err != nil || !bytes.Equal(payload, encoded) {
		t.Fatal("signed delegation payload changed")
	}
	signature, err = base64.RawURLEncoding.DecodeString(parts[1])
	if err != nil || !ed25519.Verify(key.Public().(ed25519.PublicKey), payload, signature) {
		t.Fatal("signed delegation envelope invalid")
	}
}
