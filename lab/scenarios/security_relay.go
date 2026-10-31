package scenarios

import (
	"crypto/ed25519"
	"crypto/rand"
	"encoding/base64"
	"os"
	"path/filepath"

	"github.com/JohanSardar/apple-bridge/internal/relay"
)

func securityRelayProofs(add func(string, bool)) {
	dir, err := os.MkdirTemp("", "ab-test-security-relay-")
	add("relay_fixture", err == nil)
	if err != nil {
		return
	}
	defer os.RemoveAll(dir)
	store, err := relay.Open(filepath.Join(dir, "state.json"), "relay.example", relay.Limits{Connections: 2, BytesPerSecond: 1024, MonthlyBytes: 4096})
	if err != nil {
		add("relay_fixture_store", false)
		return
	}
	key, private, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		add("relay_fixture_key", false)
		return
	}
	nonce, _ := store.Challenge()
	enrollment := relay.Enrollment{PublicKey: base64.RawURLEncoding.EncodeToString(key), Nonce: nonce, Signature: base64.RawURLEncoding.EncodeToString(ed25519.Sign(private, relay.EnrollmentMessage("relay.example", nonce)))}
	credential, err := store.Enroll(enrollment)
	add("relay_signed_enrollment", err == nil)
	if err != nil {
		return
	}
	_, err = store.Enroll(enrollment)
	add("relay_enrollment_replay_refused", err != nil)
	proof := func(action string) relay.Proof {
		nonce, _ := store.Challenge()
		p := relay.Proof{Hostname: credential.Hostname, Credential: credential.Credential, Nonce: nonce}
		p.Signature = base64.RawURLEncoding.EncodeToString(ed25519.Sign(private, relay.ProofMessage(p, action)))
		return p
	}
	p := proof("tunnel")
	add("relay_proof_valid_once", store.Verify(p, "tunnel") && !store.Verify(p, "tunnel"))
	add("relay_proof_wrong_action_refused", !store.Verify(proof("tunnel"), "dns:value"))
	p = proof("tunnel")
	p.Hostname = "attacker.relay.example"
	add("relay_proof_wrong_device_refused", !store.Verify(p, "tunnel"))
	p = proof("tunnel")
	p.Credential = "[ab-test] wrong"
	add("relay_wrong_credential_refused", !store.Verify(p, "tunnel"))
	p = proof("tunnel")
	_, wrong, _ := ed25519.GenerateKey(rand.Reader)
	p.Signature = base64.RawURLEncoding.EncodeToString(ed25519.Sign(wrong, relay.ProofMessage(p, "tunnel")))
	add("relay_credential_without_private_key_refused", !store.Verify(p, "tunnel"))
	device, _ := store.Device(credential.Hostname)
	err = store.Policy(credential.Hostname, device.Limits, false, true)
	add("relay_revoked_credential_refused", err == nil && !store.Verify(proof("tunnel"), "tunnel"))
}
