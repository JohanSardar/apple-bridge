package trust

import (
	"context"
	"crypto/tls"
	"net/http"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/coder/websocket"
)

func dialRelay(ctx context.Context, address string) (*websocket.Conn, []byte, error) {
	u, err := relayURL(address)
	if err != nil {
		return nil, nil, err
	}
	u.Scheme, u.Path = "wss", "/v1/feed"
	transport := http.DefaultTransport.(*http.Transport).Clone()
	transport.TLSClientConfig = &tls.Config{MinVersion: tls.VersionTLS13}
	client := &http.Client{Transport: transport}
	defer client.CloseIdleConnections()
	bounded, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()
	return DialBoundRelay(bounded, client, u.String())
}
func (a *Authority) DialRelay(ctx context.Context, enrollment Enrollment) (*websocket.Conn, error) {
	if !enrollment.Remote || enrollment.Dial != "" {
		return nil, contracts.ErrInvalid
	}
	conn, binding, err := dialRelay(ctx, enrollment.Address)
	if err != nil {
		return nil, err
	}
	principal, err := a.Relay2Client(ctx, conn, enrollment.Fingerprint, nil, binding)
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
	conn, binding, err := dialRelay(ctx, code.Address)
	if err != nil {
		return err
	}
	defer conn.CloseNow()
	return a.joinRelay2(ctx, conn, code, binding)
}
