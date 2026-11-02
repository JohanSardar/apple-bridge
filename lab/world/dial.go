package world

import (
	"context"
	"crypto/x509"
	"encoding/pem"
	"net/http"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/coder/websocket"
)

func (w *World) UnpairedDialRejected(index int) error {
	endpoint := w.Replicas[index]
	certificate, err := w.execute(endpoint.Remote, "cat", filepath.Join(endpoint.Home, "identity.pem"))
	if err != nil {
		return err
	}
	block, _ := pem.Decode([]byte(certificate))
	if block == nil {
		return errUnexpectedPairing
	}
	cert, err := x509.ParseCertificate(block.Bytes)
	if err != nil {
		return err
	}
	gateway, err := w.Certificate(w.Gateway)
	if err != nil {
		return err
	}
	pinned := &http.Transport{TLSClientConfig: pinnedTLS(gateway, identityFingerprint(cert.Raw))}
	defer pinned.CloseIdleConnections()
	live := &http.Client{Transport: pinned, Timeout: 2 * time.Second}
	response, err := live.Get("https://" + w.proxy.Address() + "/v1/listener-probe")
	if err != nil {
		return err
	}
	response.Body.Close()
	if response.StatusCode != 404 {
		return errUnexpectedPairing
	}

	home := w.Dir + "/unpaired-dial"
	defer w.CleanupCredential(home)
	_, err = w.execute(false, w.bridge, "token", "create", "--home", home, "--scope", "system:read")
	if err != nil {
		return err
	}
	a, err := w.Certificate(Endpoint{Home: home})
	if err != nil {
		return err
	}
	transport := &http.Transport{TLSClientConfig: pinnedTLS(a, identityFingerprint(cert.Raw))}
	defer transport.CloseIdleConnections()
	ctx, cancel := context.WithTimeout(w.ctx, 2*time.Second)
	defer cancel()
	conn, _, err := websocket.Dial(ctx, "wss://"+w.proxy.Address()+"/v1/feed", &websocket.DialOptions{HTTPClient: &http.Client{Transport: transport}})
	if err != nil {
		return nil
	}
	conn.CloseNow()
	return errUnexpectedPairing
}

func (w *World) GatewayDialStatus() ([]contracts.DialStatus, error) {
	var result contracts.ReadResult
	_, err := Request(w.ctx, w.Gateway, "/v1/op/replica.status", struct{}{}, &result)
	if err != nil {
		return nil, err
	}
	if result.Replica == nil {
		return nil, errUnexpectedPairing
	}
	return result.Replica.DialReplicas, nil
}

func (w *World) DialPairingReplayRejected(index int) bool {
	endpoint := w.Replicas[index]
	binary := w.endpointBinary(endpoint)
	_, err := w.execute(endpoint.Remote, binary, "join", w.dialCode, "--home", endpoint.Home, "--listen", endpoint.Listen)
	return err != nil && strings.Contains(err.Error(), "unauthorized")
}
