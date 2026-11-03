package wire

import (
	"encoding/base32"
	"encoding/json"
	"strings"
	"testing"
	"time"
)

func TestPairCodeWire(t *testing.T) {
	now := time.Date(2026, 11, 3, 12, 0, 0, 0, time.UTC)
	fixture := `{"purpose":"replica","remote":true,"id":"pair-fixture","address":"https://relay.example:443/","fingerprint":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","secret":"AAECAwQFBgcICQoLDA0ODxAREhM=","expires_at":"2026-11-03T12:00:00Z"}`
	encode := func(raw string) string {
		return "ab1-" + base32.StdEncoding.WithPadding(base32.NoPadding).EncodeToString([]byte(raw))
	}
	code, err := DecodePairCode(encode(fixture), now)
	if err != nil || code.Purpose != "replica" || !code.Remote || code.ID != "pair-fixture" || code.Address != "https://relay.example:443/" || code.Fingerprint != strings.Repeat("a", 64) || len(code.Secret) != 20 || code.Secret[19] != 19 || !code.ExpiresAt.Equal(now) {
		t.Fatalf("pair wire changed: %+v %v", code, err)
	}
	if _, err = DecodePairCode(encode(fixture), now.Add(time.Nanosecond)); err == nil {
		t.Fatal("expired pair code accepted")
	}
	for _, address := range []string{"https://relay.example", "https://[2001:db8::1]:443", "https://relay.example?", "https://relay.example#"} {
		if _, err = DecodePairCode(encode(strings.Replace(fixture, "https://relay.example:443/", address, 1)), now); err != nil {
			t.Fatalf("valid relay address %q: %v", address, err)
		}
	}
	for _, address := range []string{"http://relay.example", "https:///", "https://user@relay.example", "https://relay.example/path", "https://relay.example?q=1", "https://relay.example#part", "https://relay.example:0", "https://relay.example:65536", "https://relay.example:port", "https:opaque"} {
		if _, err = DecodePairCode(encode(strings.Replace(fixture, "https://relay.example:443/", address, 1)), now); err == nil {
			t.Fatalf("invalid relay address %q accepted", address)
		}
	}
	for name, encoded := range map[string]string{
		"prefix":      "wrong-" + strings.TrimPrefix(encode(fixture), "ab1-"),
		"base32":      "ab1-!",
		"json":        encode("{"),
		"id":          encode(strings.Replace(fixture, `"id":"pair-fixture"`, `"id":""`, 1)),
		"address":     encode(strings.Replace(fixture, `"address":"https://relay.example:443/"`, `"address":""`, 1)),
		"fingerprint": encode(strings.Replace(fixture, strings.Repeat("a", 64), "short", 1)),
		"secret":      encode(strings.Replace(fixture, "AAECAwQFBgcICQoLDA0ODxAREhM=", "AA==", 1)),
		"dial":        encode(strings.Replace(fixture, `"remote":true`, `"remote":true,"dial":"forbidden"`, 1)),
	} {
		if _, err = DecodePairCode(encoded, now); err == nil {
			t.Errorf("invalid %s accepted", name)
		}
	}
	local := strings.Replace(strings.Replace(fixture, `"remote":true`, `"remote":false,"dial":"local"`, 1), "https://relay.example:443/", "127.0.0.1:1234", 1)
	if _, err = DecodePairCode(encode(local), now); err != nil {
		t.Fatalf("local code: %v", err)
	}
	raw, err := json.Marshal(code)
	if err != nil || string(raw) != fixture {
		t.Fatalf("wire roundtrip: %s %v", raw, err)
	}
}
