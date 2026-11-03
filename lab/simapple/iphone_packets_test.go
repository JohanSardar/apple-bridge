package simapple

import (
	"bytes"
	"crypto/ecdh"
	"crypto/ed25519"
	"encoding/hex"
	"encoding/json"
	"os"
	"testing"
)

func TestIPhonePacketGolden(t *testing.T) {
	raw, err := os.ReadFile("../../contracts/golden/iphone.json")
	if err != nil {
		t.Fatal(err)
	}
	var golden struct {
		Recipient string       `json:"recipient_private_hex"`
		Seed      string       `json:"signing_seed_hex"`
		Info      string       `json:"info"`
		Plaintext string       `json:"plaintext"`
		Packet    IPhonePacket `json:"packet"`
	}
	if err = json.Unmarshal(raw, &golden); err != nil {
		t.Fatal(err)
	}
	private, err := hex.DecodeString(golden.Recipient)
	if err != nil {
		t.Fatal(err)
	}
	seed, err := hex.DecodeString(golden.Seed)
	if err != nil {
		t.Fatal(err)
	}
	key, err := ecdh.X25519().NewPrivateKey(private)
	if err != nil {
		t.Fatal(err)
	}
	phone := &IPhone{Encryption: key, ID: golden.Info, Gateway: ed25519.NewKeyFromSeed(seed).Public().(ed25519.PublicKey)}
	plain, err := phone.Open(golden.Packet)
	if err != nil || string(plain) != golden.Plaintext {
		t.Fatalf("frozen Go packet: %q, %v", plain, err)
	}
	tampered := golden.Packet
	tampered.Box = bytes.Clone(tampered.Box)
	tampered.Box[0] ^= 1
	if _, err = phone.Open(tampered); err == nil {
		t.Fatal("tampered packet accepted")
	}
	phone.ID += "-other-device"
	if _, err = phone.Open(golden.Packet); err == nil {
		t.Fatal("packet accepted for another device")
	}
}
