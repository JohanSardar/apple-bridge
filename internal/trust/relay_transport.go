package trust

import (
	"context"
	"crypto/tls"
	"net/http"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/coder/websocket"
)

func dialRelay(ctx context.Context, address string) (*websocket.Conn, error) {
	u, err := relayURL(address)
	if err != nil {
		return nil, err
	}
	u.Scheme, u.Path = "wss", "/v1/feed"
	transport := http.DefaultTransport.(*http.Transport).Clone()
	transport.TLSClientConfig = &tls.Config{MinVersion: tls.VersionTLS13}
	client := &http.Client{Transport: transport}
	defer client.CloseIdleConnections()
	bounded, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()
	conn, _, err := websocket.Dial(bounded, u.String(), &websocket.DialOptions{HTTPClient: client})
	return conn, err
}
func (a *Authority) DialRelay(ctx context.Context, enrollment Enrollment) (*websocket.Conn, error) {
	if !enrollment.Remote || enrollment.Dial != "" {
		return nil, contracts.ErrInvalid
	}
	conn, err := dialRelay(ctx, enrollment.Address)
	if err != nil {
		return nil, err
	}
	principal, err := a.RelayClient(ctx, conn, enrollment.Fingerprint)
	if err != nil || principal.ID != enrollment.ReplicaID {
		conn.CloseNow()
		if err != nil {
			return nil, err
		}
		return nil, contracts.ErrUnauthorized
	}
	return conn, nil
}
func (a *Authority) joinRemote(ctx context.Context, code PairCode) error {
	conn, err := dialRelay(ctx, code.Address)
	if err != nil {
		return err
	}
	defer conn.CloseNow()
	return a.JoinRelay(ctx, conn, code)
}
