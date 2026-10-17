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
	"github.com/JohanSardar/apple-bridge/internal/trust"
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
	gateway, err := trust.Open(w.Gateway.Home)
	if err != nil {
		return err
	}
	pinned := &http.Transport{TLSClientConfig: gateway.ClientTLS(trust.Fingerprint(cert.Raw))}
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
	a, err := trust.Open(home)
	if err != nil {
		return err
	}
	transport := &http.Transport{TLSClientConfig: a.ClientTLS(trust.Fingerprint(cert.Raw))}
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
	binary := w.bridge
	if endpoint.Remote {
		binary = w.remoteBinary
	}
	_, err := w.execute(endpoint.Remote, binary, "join", w.dialCode, "--home", endpoint.Home, "--listen", endpoint.Listen)
	return err != nil && strings.Contains(err.Error(), "unauthorized")
}
