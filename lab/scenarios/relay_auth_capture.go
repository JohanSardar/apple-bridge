package scenarios

import (
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha256"
	"crypto/x509"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/coder/websocket"
	"github.com/coder/websocket/wsjson"
)

func relayOracleInput(role string, hello, challenge trust.RelayFrame) []byte {
	input := append([]byte("apple-bridge-relay/1\x00"+role+"\x00"), hello.Nonce...)
	input = append(input, challenge.Nonce...)
	for _, value := range [][]byte{hello.Certificate, challenge.Certificate, []byte(hello.PairingID), nil} {
		hash := sha256.Sum256(value)
		input = append(input, hash[:]...)
	}
	return input
}
func relayAuthDial(ctx context.Context, fixture *httptest.Server) (*websocket.Conn, error) {
	conn, _, err := websocket.Dial(ctx, strings.Replace(fixture.URL, "https:", "wss:", 1)+"/v1/feed", &websocket.DialOptions{HTTPClient: fixture.Client()})
	return conn, err
}
func relayCapture(ctx context.Context, fixture *httptest.Server, replica *trust.Authority, pin string, add func(string, bool)) {
	conn, err := relayAuthDial(ctx, fixture)
	add("capture_connection", err == nil)
	if err != nil {
		return
	}
	defer conn.CloseNow()
	nonce := make([]byte, 32)
	if _, err = rand.Read(nonce); err != nil {
		add("capture_nonce", false)
		return
	}
	hello := trust.RelayFrame{T: "AuthHello", Nonce: nonce, Certificate: replica.Certificate.Certificate[0]}
	wsjson.Write(ctx, conn, hello)
	var challenge trust.RelayFrame
	if err = wsjson.Read(ctx, conn, &challenge); err != nil {
		add("capture_challenge", false)
		return
	}
	cert, err := x509.ParseCertificate(challenge.Certificate)
	if err != nil {
		add("capture_gateway_certificate", false)
		return
	}
	key, ok := cert.PublicKey.(ed25519.PublicKey)
	add("gateway_signed_both_nonces_and_pinned_identities", ok && len(challenge.Nonce) == 32 && trust.Fingerprint(cert.Raw) == pin && ed25519.Verify(key, relayOracleInput("gateway", hello, challenge), challenge.Signature))
	proof := trust.RelayFrame{T: "AuthProof", Signature: ed25519.Sign(replica.Certificate.PrivateKey.(ed25519.PrivateKey), relayOracleInput("replica", hello, challenge))}
	wsjson.Write(ctx, conn, proof)
	var accepted trust.RelayFrame
	err = wsjson.Read(ctx, conn, &accepted)
	add("independent_replica_signature_accepted", err == nil && accepted.T == "Authenticated")

	replay, err := relayAuthDial(ctx, fixture)
	add("replay_connection", err == nil)
	if err != nil {
		return
	}
	defer replay.CloseNow()
	wsjson.Write(ctx, replay, hello)
	var fresh trust.RelayFrame
	err = wsjson.Read(ctx, replay, &fresh)
	add("fresh_server_nonce_on_reused_client_nonce", err == nil && string(fresh.Nonce) != string(challenge.Nonce))
	wsjson.Write(ctx, replay, proof)
	add("captured_auth_frame_replay_rejected", relayClosed(ctx, replay))
	proofSent := make(chan bool, 1)
	fake := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			proofSent <- false
			return
		}
		defer conn.CloseNow()
		bounded, cancel := context.WithTimeout(r.Context(), 2*time.Second)
		defer cancel()
		var message trust.RelayFrame
		if wsjson.Read(bounded, conn, &message) != nil {
			proofSent <- false
			return
		}
		wsjson.Write(bounded, conn, challenge)
		err = wsjson.Read(bounded, conn, &message)
		proofSent <- err == nil && message.T == "AuthProof"
	}))
	defer fake.Close()
	forged, err := relayAuthDial(ctx, fake)
	add("gateway_replay_tls_connection", err == nil)
	if err == nil {
		_, err = replica.RelayClient(ctx, forged, pin)
		forged.CloseNow()
		add("replayed_gateway_signature_rejected_before_replica_proof", err != nil && !<-proofSent)
	}
}
func relayClosed(ctx context.Context, conn *websocket.Conn) bool {
	bounded, cancel := context.WithTimeout(ctx, time.Second)
	defer cancel()
	var frame trust.RelayFrame
	err := wsjson.Read(bounded, conn, &frame)
	return err != nil && bounded.Err() == nil
}
func relayPrincipalID(p trust.Principal) bool {
	return strings.HasPrefix(p.ID, "replica:") && p.Kind == "replica" && len(p.Grants) > 0 && p.Revision > 0
}
func relayRejectHello(ctx context.Context, fixture *httptest.Server) bool {
	conn, err := relayAuthDial(ctx, fixture)
	if err != nil {
		return false
	}
	defer conn.CloseNow()
	wsjson.Write(ctx, conn, contracts.FeedFrame{T: "Hello", Protocol: contracts.FeedProtocol, ReplicaID: "replica:unverified"})
	return relayClosed(ctx, conn)
}

func relayRejectBinary(ctx context.Context, fixture *httptest.Server, replica *trust.Authority) bool {
	conn, err := relayAuthDial(ctx, fixture)
	if err != nil {
		return false
	}
	defer conn.CloseNow()
	nonce := make([]byte, 32)
	if _, err = rand.Read(nonce); err != nil {
		return false
	}
	data, err := json.Marshal(trust.RelayFrame{T: "AuthHello", Nonce: nonce, Certificate: replica.Certificate.Certificate[0]})
	if err != nil || conn.Write(ctx, websocket.MessageBinary, data) != nil {
		return false
	}
	return relayClosed(ctx, conn)
}
