package trust

import (
	"context"
	"crypto/x509"
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/coder/websocket"
	"github.com/coder/websocket/wsjson"
)

func (a *Authority) RelayServer(ctx context.Context, conn *websocket.Conn) (Principal, *x509.Certificate, error) {
	ctx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	conn.SetReadLimit(8192)
	var hello RelayFrame
	if err := relayRead(ctx, conn, &hello); err != nil {
		return Principal{}, nil, err
	}
	cert, err := relayCertificate(hello)
	if err != nil || hello.T != "AuthHello" || hello.PairingID != "" {
		return Principal{}, nil, contracts.ErrUnauthorized
	}
	nonce, err := relayNonce()
	if err != nil {
		return Principal{}, nil, err
	}
	challenge := RelayFrame{T: "AuthChallenge", Nonce: nonce, Certificate: a.Certificate.Certificate[0]}
	challenge.Signature, err = a.relaySign("gateway", hello, challenge, nil)
	if err != nil {
		return Principal{}, nil, err
	}
	if err = wsjson.Write(ctx, conn, challenge); err != nil {
		return Principal{}, nil, err
	}
	var proof RelayFrame
	if err = relayRead(ctx, conn, &proof); err != nil {
		return Principal{}, nil, err
	}
	if proof.T != "AuthProof" || relayVerify(cert, proof.Signature, "replica", hello, challenge, nil) != nil {
		return Principal{}, nil, contracts.ErrUnauthorized
	}
	principal, err := a.AuthenticateCertificate(cert)
	if err != nil {
		return Principal{}, nil, err
	}
	raw, err := json.Marshal(principal)
	if err != nil {
		return Principal{}, nil, err
	}
	accepted := RelayFrame{T: "Authenticated", Principal: raw}
	accepted.Signature, err = a.relaySign("accepted", hello, challenge, raw)
	if err != nil {
		return Principal{}, nil, err
	}
	if err = wsjson.Write(ctx, conn, accepted); err != nil {
		return Principal{}, nil, err
	}
	return principal, cert, nil
}
