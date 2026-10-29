package relay

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"io"
	"math/big"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

func TestAPNsBackgroundRequest(t *testing.T) {
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	requests := 0
	server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		requests++
		if r.URL.Path != "/3/device/"+strings.Repeat("a", 64) || r.Header.Get("apns-topic") != "com.johansardar.applebridge.ios" || r.Header.Get("apns-push-type") != "background" || r.Header.Get("apns-priority") != "5" {
			t.Error("APNs routing headers")
		}
		parts := strings.Split(strings.TrimPrefix(r.Header.Get("Authorization"), "bearer "), ".")
		if len(parts) != 3 {
			t.Error("JWT shape")
			return
		}
		signature, err := base64.RawURLEncoding.DecodeString(parts[2])
		if err != nil || len(signature) != 64 {
			t.Error("JWT signature shape")
			return
		}
		hash := sha256.Sum256([]byte(parts[0] + "." + parts[1]))
		if !ecdsa.Verify(&key.PublicKey, hash[:], new(big.Int).SetBytes(signature[:32]), new(big.Int).SetBytes(signature[32:])) {
			t.Error("JWT signature")
		}
		claims, _ := base64.RawURLEncoding.DecodeString(parts[1])
		var c struct {
			Issuer string `json:"iss"`
			At     int64  `json:"iat"`
		}
		json.Unmarshal(claims, &c)
		if c.Issuer != "AX3C2B78YD" || time.Since(time.Unix(c.At, 0)) > time.Minute {
			t.Error("JWT claims")
		}
		raw, _ := io.ReadAll(r.Body)
		var payload struct {
			APS    map[string]int  `json:"aps"`
			Bridge json.RawMessage `json:"bridge"`
		}
		if json.Unmarshal(raw, &payload) != nil || payload.APS["content-available"] != 1 || len(payload.Bridge) == 0 {
			t.Error("silent encrypted payload")
		}
	}))
	defer server.Close()
	provider := &APNs{Key: key, KeyID: "L2854KRF5Z", Team: "AX3C2B78YD", Endpoint: server.URL, Client: server.Client()}
	if err := provider.Send(t.Context(), strings.Repeat("a", 64), json.RawMessage(`{"encrypted":true}`), time.Now().Add(time.Minute)); err != nil {
		t.Fatal(err)
	}
	if requests != 1 {
		t.Fatal("push not sent")
	}
}
