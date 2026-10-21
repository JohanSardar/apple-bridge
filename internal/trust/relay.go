package trust

import (
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha256"
	"crypto/x509"
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/coder/websocket"
)

type RelayFrame struct {
	T           string          `json:"t"`
	Nonce       []byte          `json:"nonce,omitempty"`
	Certificate []byte          `json:"certificate,omitempty"`
	Signature   []byte          `json:"signature,omitempty"`
	PairingID   string          `json:"pairing_id,omitempty"`
	Proof       []byte          `json:"proof,omitempty"`
	Principal   json.RawMessage `json:"principal,omitempty"`
}

func relayNonce() ([]byte, error) {
	value := make([]byte, 32)
	_, err := rand.Read(value)
	return value, err
}
func relayInput(role string, hello, challenge RelayFrame, principal json.RawMessage) []byte {
	value := []byte("apple-bridge-relay/1\x00" + role + "\x00")
	value = append(value, hello.Nonce...)
	value = append(value, challenge.Nonce...)
	for _, part := range [][]byte{hello.Certificate, challenge.Certificate, []byte(hello.PairingID), principal} {
		hash := sha256.Sum256(part)
		value = append(value, hash[:]...)
	}
	return value
}
func relayCertificate(frame RelayFrame) (*x509.Certificate, error) {
	if len(frame.Nonce) != 32 || len(frame.Certificate) > 4096 {
		return nil, contracts.ErrUnauthorized
	}
	cert, err := x509.ParseCertificate(frame.Certificate)
	if err != nil {
		return nil, contracts.ErrUnauthorized
	}
	if _, ok := cert.PublicKey.(ed25519.PublicKey); !ok {
		return nil, contracts.ErrUnauthorized
	}
	if time.Now().Before(cert.NotBefore) || time.Now().After(cert.NotAfter) {
		return nil, contracts.ErrExpired
	}
	return cert, nil
}
func (a *Authority) relaySign(role string, hello, challenge RelayFrame, principal json.RawMessage) ([]byte, error) {
	key, ok := a.Certificate.PrivateKey.(ed25519.PrivateKey)
	if !ok {
		return nil, contracts.ErrCorrupt
	}
	return ed25519.Sign(key, relayInput(role, hello, challenge, principal)), nil
}
func relayVerify(cert *x509.Certificate, signature []byte, role string, hello, challenge RelayFrame, principal json.RawMessage) error {
	key, ok := cert.PublicKey.(ed25519.PublicKey)
	if !ok || !ed25519.Verify(key, relayInput(role, hello, challenge, principal), signature) {
		return contracts.ErrUnauthorized
	}
	return nil
}

func relayRead(ctx context.Context, conn *websocket.Conn, frame *RelayFrame) error {
	kind, data, err := conn.Read(ctx)
	if err != nil {
		return err
	}
	if kind != websocket.MessageText || len(data) > 8192 {
		return contracts.ErrInvalid
	}
	return contracts.DecodeInput(data, frame)
}
