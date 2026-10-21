package trust

import (
	"context"
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/coder/websocket"
	"github.com/coder/websocket/wsjson"
)

func (a *Authority) RelayClient(ctx context.Context, conn *websocket.Conn, fingerprint string) (Principal, error) {
	return a.relayClient(ctx, conn, fingerprint, nil)
}
func (a *Authority) relayClient(ctx context.Context, conn *websocket.Conn, fingerprint string, pairing *PairCode) (Principal, error) {
	ctx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	conn.SetReadLimit(8192)
	nonce, err := relayNonce()
	if err != nil {
		return Principal{}, err
	}
	hello := RelayFrame{T: "AuthHello", Nonce: nonce, Certificate: a.Certificate.Certificate[0]}
	if pairing != nil {
		hello.PairingID = pairing.ID
	}
	if err = wsjson.Write(ctx, conn, hello); err != nil {
		return Principal{}, err
	}
	var challenge RelayFrame
	if err = relayRead(ctx, conn, &challenge); err != nil {
		return Principal{}, err
	}
	cert, err := relayCertificate(challenge)
	if err != nil || challenge.T != "AuthChallenge" || Fingerprint(challenge.Certificate) != fingerprint {
		return Principal{}, contracts.ErrUnauthorized
	}
	if err = relayVerify(cert, challenge.Signature, "gateway", hello, challenge, nil); err != nil {
		return Principal{}, err
	}
	proof := RelayFrame{T: "AuthProof"}
	if pairing != nil {
		proof.Proof = PairProof(pairing.Secret, a.Certificate.Certificate[0])
	}
	proof.Signature, err = a.relaySign("replica", hello, challenge, nil)
	if err != nil {
		return Principal{}, err
	}
	if err = wsjson.Write(ctx, conn, proof); err != nil {
		return Principal{}, err
	}
	var accepted RelayFrame
	if err = relayRead(ctx, conn, &accepted); err != nil {
		return Principal{}, err
	}
	if accepted.T != "Authenticated" || relayVerify(cert, accepted.Signature, "accepted", hello, challenge, accepted.Principal) != nil {
		return Principal{}, contracts.ErrUnauthorized
	}
	var principal Principal
	if json.Unmarshal(accepted.Principal, &principal) != nil || principal.Kind != "replica" || principal.Fingerprint != a.Fingerprint {
		return Principal{}, contracts.ErrUnauthorized
	}
	return principal, nil
}
