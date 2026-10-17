package world

import (
	"context"
	"net/http"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/coder/websocket"
)

func (w *World) RevokedDialRejected(index int) error {
	home := w.Dir + "/unpaired-dial"
	defer w.CleanupCredential(home)
	a, err := trust.Open(home)
	if err != nil {
		return err
	}
	transport := &http.Transport{TLSClientConfig: a.ClientTLS("")}
	defer transport.CloseIdleConnections()
	transport.TLSClientConfig.VerifyConnection = nil
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
