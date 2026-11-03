package main

import (
	"bytes"
	"crypto/ecdh"
	"crypto/ed25519"
	"crypto/hmac"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/iphone"
	"github.com/JohanSardar/apple-bridge/internal/relay"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"os"
	"time"
)

type input struct {
	Payload    json.RawMessage         `json:"payload"`
	Batch      contracts.HealthBatch   `json:"batch"`
	Command    contracts.Command       `json:"command"`
	Now        int64                   `json:"now"`
	At         int64                   `json:"at"`
	Pair       trust.IPhonePairRequest `json:"pair"`
	Op         string                  `json:"op"`
	Home       string                  `json:"home"`
	Code       string                  `json:"code"`
	ID         string                  `json:"id"`
	Path       string                  `json:"path"`
	Request    trust.IPhoneRequest     `json:"request"`
	Packet     iphone.Packet           `json:"packet"`
	GatewayKey []byte                  `json:"gateway_key"`
}

func run(in input) (any, error) {
	key := ed25519.NewKeyFromSeed(bytes.Repeat([]byte{42}, 32))
	encryption, err := ecdh.X25519().NewPrivateKey(bytes.Repeat([]byte{17}, 32))
	if err != nil {
		return nil, err
	}
	switch in.Op {
	case "health-digest":
		raw, err := json.Marshal(in.Batch)
		digest := sha256.Sum256(raw)
		return map[string]string{"digest": hex.EncodeToString(digest[:]), "raw": string(raw)}, err
	case "request":
		at := in.At
		if at == 0 {
			at = time.Now().Unix()
		}
		payload := []byte(`{"command_id":"synthetic-command"}`)
		if len(in.Payload) != 0 {
			payload = in.Payload
		}
		return trust.IPhoneRequest{DeviceID: in.ID, At: at, Payload: payload, Signature: ed25519.Sign(key, trust.IPhoneMessage(in.Path, at, payload))}, nil
	case "pair-request":
		code, err := trust.DecodePairCode(in.Code)
		if err != nil {
			return nil, err
		}
		public := key.Public().(ed25519.PublicKey)
		message := trust.IPhonePairMessage(code.ID, public, encryption.PublicKey().Bytes())
		hash := sha256.Sum256(code.Secret)
		mac := hmac.New(sha256.New, hash[:])
		mac.Write(message)
		return trust.IPhonePairRequest{ID: code.ID, SigningKey: public, EncryptionKey: encryption.PublicKey().Bytes(), Proof: mac.Sum(nil), Signature: ed25519.Sign(key, message)}, nil
	case "push-json":
		raw, err := json.Marshal(relay.IPhonePush{Hostname: "fixture.example", Token: "synthetic-token", Packet: in.Packet, Expires: 1800000000})
		return map[string]string{"raw": string(raw)}, err
	case "open":
		plain, err := iphone.Open(encryption, in.ID, in.Packet, in.GatewayKey)
		return map[string]string{"plaintext": string(plain)}, err
	}
	a, err := trust.Open(in.Home)
	if err != nil {
		return nil, err
	}
	switch in.Op {
	case "code":
		code, err := a.PairIPhoneCode("https://fixture.example", false, time.Minute)
		return map[string]string{"code": code}, err
	case "pair":
		return a.PairIPhone(in.Pair)
	case "verify":
		p, err := a.VerifyIPhone(in.Path, in.Request)
		return map[string]string{"id": p.ID}, err
	case "live":
		p, err := a.Principal(in.ID)
		if err != nil {
			return nil, err
		}
		token, raw, err := iphone.LivePayload(p, in.Command, time.Unix(in.Now, 0))
		return map[string]string{"token": token, "raw": string(raw)}, err
	case "list":
		return a.IPhones()
	case "revoke":
		err := a.Revoke(in.ID)
		return map[string]bool{"revoked": err == nil}, err
	case "principal":
		return a.Principal(in.ID)
	default:
		return nil, fmt.Errorf("op")
	}
}
func main() {
	os.Setenv("APPLE_BRIDGE_SECRET_STORE", "file")
	var in input
	if err := json.NewDecoder(os.Stdin).Decode(&in); err != nil {
		panic(err)
	}
	out, err := run(in)
	if err != nil {
		out = map[string]string{"error": err.Error()}
	}
	if err := json.NewEncoder(os.Stdout).Encode(out); err != nil {
		panic(err)
	}
}
