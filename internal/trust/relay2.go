package trust

import (
	"context"
	"crypto/ed25519"
	"crypto/sha256"
	"crypto/tls"
	"crypto/x509"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/coder/websocket"
	"github.com/coder/websocket/wsjson"
	"net/http"
	"time"
)

type Relay2Frame struct {
	Version      string          `json:"version"`
	Capabilities string          `json:"capabilities"`
	T            string          `json:"t"`
	Nonce        []byte          `json:"nonce,omitempty"`
	Certificate  []byte          `json:"certificate,omitempty"`
	Signature    []byte          `json:"signature,omitempty"`
	PairingID    string          `json:"pairing_id,omitempty"`
	Proof        []byte          `json:"proof,omitempty"`
	Principal    json.RawMessage `json:"principal,omitempty"`
}

func relay2Input(role string, hello, challenge Relay2Frame, principal json.RawMessage, binding []byte) []byte {
	value := []byte("apple-bridge-relay/2\x00" + role + "\x00")
	value = append(value, []byte("relay/2\x00tls-exporter,feed/1\x00")...)
	value = append(value, binding...)
	value = append(value, hello.Nonce...)
	value = append(value, challenge.Nonce...)
	for _, part := range [][]byte{hello.Certificate, challenge.Certificate, []byte(hello.PairingID), principal} {
		hash := sha256.Sum256(part)
		value = append(value, hash[:]...)
	}
	return value
}
func relay2Certificate(frame Relay2Frame) (*x509.Certificate, error) {
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
func (a *Authority) relay2Sign(role string, hello, challenge Relay2Frame, principal json.RawMessage, binding []byte) ([]byte, error) {
	key, ok := a.Certificate.PrivateKey.(ed25519.PrivateKey)
	if !ok {
		return nil, contracts.ErrCorrupt
	}
	return ed25519.Sign(key, relay2Input(role, hello, challenge, principal, binding)), nil
}
func relay2Verify(cert *x509.Certificate, signature []byte, role string, hello, challenge Relay2Frame, principal json.RawMessage, binding []byte) error {
	key, ok := cert.PublicKey.(ed25519.PublicKey)
	if !ok || !ed25519.Verify(key, relay2Input(role, hello, challenge, principal, binding), signature) {
		return contracts.ErrUnauthorized
	}
	return nil
}

func relay2Read(ctx context.Context, conn *websocket.Conn, frame *Relay2Frame) error {
	kind, data, err := conn.Read(ctx)
	if err != nil {
		return err
	}
	if kind != websocket.MessageText || len(data) > 8192 {
		return contracts.ErrInvalid
	}
	if err := contracts.DecodeInput(data, frame); err != nil {
		return err
	}
	if frame.Version != "relay/2" || frame.Capabilities != "tls-exporter,feed/1" {
		return contracts.ErrUnauthorized
	}
	return nil
}
func (a *Authority) Relay2Client(ctx context.Context, conn *websocket.Conn, fingerprint string, pairing *PairCode, binding []byte) (Principal, error) {
	if len(binding) != 32 {
		return Principal{}, contracts.ErrUnauthorized
	}
	ctx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	conn.SetReadLimit(8192)
	nonce, err := relayNonce()
	if err != nil {
		return Principal{}, err
	}
	hello := Relay2Frame{Version: "relay/2", Capabilities: "tls-exporter,feed/1", T: "AuthHello", Nonce: nonce, Certificate: a.Certificate.Certificate[0]}
	if pairing != nil {
		hello.PairingID = pairing.ID
	}
	if err = wsjson.Write(ctx, conn, hello); err != nil {
		return Principal{}, err
	}
	var challenge Relay2Frame
	if err = relay2Read(ctx, conn, &challenge); err != nil {
		return Principal{}, err
	}
	cert, err := relay2Certificate(challenge)
	if err != nil || challenge.T != "AuthChallenge" || Fingerprint(challenge.Certificate) != fingerprint {
		return Principal{}, contracts.ErrUnauthorized
	}
	if err = relay2Verify(cert, challenge.Signature, "gateway", hello, challenge, nil, binding); err != nil {
		return Principal{}, err
	}
	proof := Relay2Frame{Version: "relay/2", Capabilities: "tls-exporter,feed/1", T: "AuthProof"}
	if pairing != nil {
		proof.Proof = PairProof(pairing.Secret, a.Certificate.Certificate[0])
	}
	proof.Signature, err = a.relay2Sign("replica", hello, challenge, nil, binding)
	if err != nil {
		return Principal{}, err
	}
	if err = wsjson.Write(ctx, conn, proof); err != nil {
		return Principal{}, err
	}
	var accepted Relay2Frame
	if err = relay2Read(ctx, conn, &accepted); err != nil {
		return Principal{}, err
	}
	if accepted.T != "Authenticated" || relay2Verify(cert, accepted.Signature, "accepted", hello, challenge, accepted.Principal, binding) != nil {
		return Principal{}, contracts.ErrUnauthorized
	}
	var principal Principal
	if json.Unmarshal(accepted.Principal, &principal) != nil || principal.Kind != "replica" || principal.Fingerprint != a.Fingerprint {
		return Principal{}, contracts.ErrUnauthorized
	}
	return principal, nil
}
func (a *Authority) Relay2Server(ctx context.Context, conn *websocket.Conn, binding []byte) (Principal, *x509.Certificate, error) {
	if len(binding) != 32 {
		return Principal{}, nil, contracts.ErrUnauthorized
	}
	ctx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	conn.SetReadLimit(8192)
	var hello Relay2Frame
	if err := relay2Read(ctx, conn, &hello); err != nil {
		return Principal{}, nil, err
	}
	return a.relay2ServerHello(ctx, conn, hello, binding)
}
func (a *Authority) relay2ServerHello(ctx context.Context, conn *websocket.Conn, hello Relay2Frame, binding []byte) (Principal, *x509.Certificate, error) {
	cert, err := relay2Certificate(hello)
	if err != nil || hello.T != "AuthHello" {
		return Principal{}, nil, contracts.ErrUnauthorized
	}
	nonce, err := relayNonce()
	if err != nil {
		return Principal{}, nil, err
	}
	challenge := Relay2Frame{Version: "relay/2", Capabilities: "tls-exporter,feed/1", T: "AuthChallenge", Nonce: nonce, Certificate: a.Certificate.Certificate[0]}
	challenge.Signature, err = a.relay2Sign("gateway", hello, challenge, nil, binding)
	if err != nil {
		return Principal{}, nil, err
	}
	if err = wsjson.Write(ctx, conn, challenge); err != nil {
		return Principal{}, nil, err
	}
	var proof Relay2Frame
	if err = relay2Read(ctx, conn, &proof); err != nil {
		return Principal{}, nil, err
	}
	if proof.T != "AuthProof" || relay2Verify(cert, proof.Signature, "replica", hello, challenge, nil, binding) != nil {
		return Principal{}, nil, contracts.ErrUnauthorized
	}
	var principal Principal
	if hello.PairingID == "" {
		principal, err = a.AuthenticateCertificate(cert)
	} else {
		principal, err = a.acceptPair(JoinRequest{ID: hello.PairingID, Proof: proof.Proof}, cert, false)
	}
	if err != nil {
		return Principal{}, nil, err
	}
	if principal, err = a.markRelay2(principal); err != nil {
		return Principal{}, nil, err
	}
	raw, err := json.Marshal(principal)
	if err != nil {
		return Principal{}, nil, err
	}
	accepted := Relay2Frame{Version: "relay/2", Capabilities: "tls-exporter,feed/1", T: "Authenticated", Principal: raw}
	accepted.Signature, err = a.relay2Sign("accepted", hello, challenge, raw, binding)
	if err != nil {
		return Principal{}, nil, err
	}
	if err = wsjson.Write(ctx, conn, accepted); err != nil {
		return Principal{}, nil, err
	}
	return principal, cert, nil
}

func RelayBinding(state *tls.ConnectionState) ([]byte, error) {
	if state == nil || !state.HandshakeComplete || state.Version != tls.VersionTLS13 {
		return nil, contracts.ErrUnauthorized
	}
	return state.ExportKeyingMaterial("EXPORTER-Channel-Binding", []byte{}, 32)
}
func DialBoundRelay(ctx context.Context, client *http.Client, address string) (*websocket.Conn, []byte, error) {
	conn, response, err := websocket.Dial(ctx, address, &websocket.DialOptions{HTTPClient: client})
	if err != nil {
		return nil, nil, err
	}
	binding, err := RelayBinding(response.TLS)
	if err != nil {
		conn.CloseNow()
		return nil, nil, err
	}
	return conn, binding, nil
}
