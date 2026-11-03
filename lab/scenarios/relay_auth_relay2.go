package scenarios

import (
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha256"
	"crypto/x509"
	"net/http/httptest"
	"strings"

	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/coder/websocket"
	"github.com/coder/websocket/wsjson"
)

const relay2Capabilities = "tls-exporter,feed/1"

func relay2OracleInput(role string, hello, challenge trust.Relay2Frame, principal []byte, binding []byte) []byte {
	input := append([]byte("apple-bridge-relay/2\x00"+role+"\x00relay/2\x00"+relay2Capabilities+"\x00"), binding...)
	input = append(input, hello.Nonce...)
	input = append(input, challenge.Nonce...)
	for _, value := range [][]byte{hello.Certificate, challenge.Certificate, []byte(hello.PairingID), principal} {
		hash := sha256.Sum256(value)
		input = append(input, hash[:]...)
	}
	return input
}
func relay2AuthDial(ctx context.Context, fixture *httptest.Server) (*websocket.Conn, []byte, error) {
	return trust.DialBoundRelay(ctx, fixture.Client(), strings.Replace(fixture.URL, "https:", "wss:", 1)+"/v1/feed")
}
func relay2Hello(replica *trust.Authority) (trust.Relay2Frame, error) {
	nonce := make([]byte, 32)
	if _, err := rand.Read(nonce); err != nil {
		return trust.Relay2Frame{}, err
	}
	return trust.Relay2Frame{Version: "relay/2", Capabilities: relay2Capabilities, T: "AuthHello", Nonce: nonce, Certificate: replica.Certificate.Certificate[0]}, nil
}
func relay2Variant(ctx context.Context, fixture *httptest.Server, gateway, replica, stranger *trust.Authority, add func(string, bool)) {
	conn, binding, err := relay2AuthDial(ctx, fixture)
	add("relay2_bound_tls_connection", err == nil && len(binding) == 32)
	if err != nil {
		return
	}
	principal, err := replica.Relay2Client(ctx, conn, gateway.Fingerprint, nil, binding)
	add("relay2_mutual_device_key_authentication", err == nil && relayPrincipalID(principal))
	conn.CloseNow()

	capture, captureBinding, err := relay2AuthDial(ctx, fixture)
	add("relay2_capture_connection", err == nil)
	if err != nil {
		return
	}
	defer capture.CloseNow()
	hello, err := relay2Hello(replica)
	if err != nil {
		add("relay2_capture_nonce", false)
		return
	}
	wsjson.Write(ctx, capture, hello)
	var challenge trust.Relay2Frame
	if err = wsjson.Read(ctx, capture, &challenge); err != nil {
		add("relay2_capture_challenge", false)
		return
	}
	cert, err := x509.ParseCertificate(challenge.Certificate)
	if err != nil {
		add("relay2_capture_gateway_certificate", false)
		return
	}
	key, ok := cert.PublicKey.(ed25519.PublicKey)
	add("relay2_gateway_signed_binding_nonces_and_pinned_identities", ok && challenge.Version == "relay/2" && len(challenge.Nonce) == 32 && trust.Fingerprint(cert.Raw) == gateway.Fingerprint && ed25519.Verify(key, relay2OracleInput("gateway", hello, challenge, nil, captureBinding), challenge.Signature))
	add("relay2_gateway_signature_not_valid_without_binding", ok && !ed25519.Verify(key, relay2OracleInput("gateway", hello, challenge, nil, make([]byte, 32)), challenge.Signature))
	proof := trust.Relay2Frame{Version: "relay/2", Capabilities: relay2Capabilities, T: "AuthProof", Signature: ed25519.Sign(replica.Certificate.PrivateKey.(ed25519.PrivateKey), relay2OracleInput("replica", hello, challenge, nil, captureBinding))}
	wsjson.Write(ctx, capture, proof)
	var accepted trust.Relay2Frame
	err = wsjson.Read(ctx, capture, &accepted)
	add("relay2_independent_replica_signature_accepted", err == nil && accepted.T == "Authenticated" && ed25519.Verify(key, relay2OracleInput("accepted", hello, challenge, accepted.Principal, captureBinding), accepted.Signature))

	replay, _, err := relay2AuthDial(ctx, fixture)
	add("relay2_replay_connection", err == nil)
	if err != nil {
		return
	}
	defer replay.CloseNow()
	wsjson.Write(ctx, replay, hello)
	var fresh trust.Relay2Frame
	err = wsjson.Read(ctx, replay, &fresh)
	add("relay2_fresh_server_nonce_on_reused_client_nonce", err == nil && string(fresh.Nonce) != string(challenge.Nonce))
	wsjson.Write(ctx, replay, proof)
	add("relay2_captured_proof_replay_on_other_connection_rejected", relayClosed(ctx, replay))

	wrongPin, pinBinding, err := relay2AuthDial(ctx, fixture)
	if err == nil {
		_, err = replica.Relay2Client(ctx, wrongPin, strings.Repeat("0", 64), nil, pinBinding)
		wrongPin.CloseNow()
	}
	add("relay2_wrong_pin_rejected", err != nil)
	unknown, unknownBinding, err := relay2AuthDial(ctx, fixture)
	if err == nil {
		_, err = stranger.Relay2Client(ctx, unknown, gateway.Fingerprint, nil, unknownBinding)
		unknown.CloseNow()
	}
	add("relay2_unpaired_device_key_rejected", err != nil)
	legacy, err := relayAuthDial(ctx, fixture)
	if err == nil {
		_, err = replica.RelayClient(ctx, legacy, gateway.Fingerprint)
		legacy.CloseNow()
	}
	add("relay1_rejected_for_peer_that_authenticated_with_relay2", err != nil)
}
