package iphone

import (
	"bytes"
	"crypto/ecdh"
	"crypto/ed25519"
	"encoding/hex"
	"encoding/json"
	"testing"

	"github.com/JohanSardar/apple-bridge/contracts/golden"
)

func TestPacketGolden(t *testing.T) {
	recipient, e := ecdh.X25519().NewPrivateKey(bytes.Repeat([]byte{17}, 32))
	if e != nil {
		t.Fatal(e)
	}
	ephemeral, e := ecdh.X25519().NewPrivateKey(bytes.Repeat([]byte{23}, 32))
	if e != nil {
		t.Fatal(e)
	}
	secret, e := ephemeral.ECDH(recipient.PublicKey())
	if e != nil {
		t.Fatal(e)
	}
	aead, e := sharedCipher(secret, "device-fixture")
	if e != nil {
		t.Fatal(e)
	}
	nonce := bytes.Repeat([]byte{31}, aead.NonceSize())
	plain := []byte(`{"command_id":"cmd-fixture","op":"iphone.ping","args":{}}`)
	key := ed25519.NewKeyFromSeed(bytes.Repeat([]byte{42}, 32))
	packet := Packet{EphemeralKey: ephemeral.PublicKey().Bytes(), Box: aead.Seal(append([]byte{}, nonce...), nonce, plain, nil)}
	packet.Signature = ed25519.Sign(key, PacketMessage(packet))
	opened, e := Open(recipient, "device-fixture", packet, key.Public().(ed25519.PublicKey))
	if e != nil || !bytes.Equal(opened, plain) {
		t.Fatalf("open %v", e)
	}
	if _, e = Open(recipient, "wrong-device", packet, key.Public().(ed25519.PublicKey)); e == nil {
		t.Fatal("device binding lost")
	}
	b, e := json.MarshalIndent(map[string]any{"impl": "go", "salt": "apple-bridge-iphone/1", "info": "device-fixture", "recipient_private_hex": hex.EncodeToString(recipient.Bytes()), "ephemeral_private_hex": hex.EncodeToString(ephemeral.Bytes()), "signing_seed_hex": hex.EncodeToString(bytes.Repeat([]byte{42}, 32)), "plaintext": string(plain), "packet": packet}, "", "  ")
	if e != nil {
		t.Fatal(e)
	}
	golden.Check(t, "iphone.json", append(b, '\n'))
}
