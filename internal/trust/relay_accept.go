package trust

import (
	"context"
	"crypto/x509"
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/coder/websocket"
)

func (a *Authority) RelayAccept(ctx context.Context, conn *websocket.Conn, binding []byte) (Principal, *x509.Certificate, error) {
	ctx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	conn.SetReadLimit(8192)
	kind, data, err := conn.Read(ctx)
	if err != nil {
		return Principal{}, nil, err
	}
	if kind != websocket.MessageText || len(data) > 8192 {
		return Principal{}, nil, contracts.ErrInvalid
	}
	var probe map[string]json.RawMessage
	if json.Unmarshal(data, &probe) != nil {
		return Principal{}, nil, contracts.ErrInvalid
	}
	if _, versioned := probe["version"]; versioned {
		if len(binding) != 32 {
			return Principal{}, nil, contracts.ErrUnauthorized
		}
		var hello Relay2Frame
		if err = contracts.DecodeInput(data, &hello); err != nil {
			return Principal{}, nil, err
		}
		if hello.Version != "relay/2" || hello.Capabilities != "tls-exporter,feed/1" {
			return Principal{}, nil, contracts.ErrUnauthorized
		}
		return a.relay2ServerHello(ctx, conn, hello, binding)
	}
	var hello RelayFrame
	if err = contracts.DecodeInput(data, &hello); err != nil {
		return Principal{}, nil, err
	}
	return a.relayServerHello(ctx, conn, hello)
}
