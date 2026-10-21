package main

import (
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha256"
	"crypto/x509"
	"encoding/json"
	"errors"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/coder/websocket/wsjson"
	"os"
	"time"
)

func oracle(role string, hello, challenge trust.RelayFrame, principal []byte) []byte {
	input := append([]byte("apple-bridge-relay/1\x00"+role+"\x00"), hello.Nonce...)
	input = append(input, challenge.Nonce...)
	for _, part := range [][]byte{hello.Certificate, challenge.Certificate, []byte(hello.PairingID), principal} {
		hash := sha256.Sum256(part)
		input = append(input, hash[:]...)
	}
	return input
}
func replay(ctx context.Context) error {
	ctx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	var in input
	if err := json.NewDecoder(os.Stdin).Decode(&in); err != nil {
		return err
	}
	authority, err := trust.Open(in.Home)
	if err != nil {
		return err
	}
	enrollment, err := authority.Enrollment()
	if err != nil || enrollment == nil {
		return fmt.Errorf("enrollment missing")
	}
	conn, err := publicDial(ctx, in, *enrollment)
	if err != nil {
		return err
	}
	defer conn.CloseNow()
	nonce := make([]byte, 32)
	if _, err = rand.Read(nonce); err != nil {
		return err
	}
	hello := trust.RelayFrame{T: "AuthHello", Nonce: nonce, Certificate: authority.Certificate.Certificate[0]}
	if err = wsjson.Write(ctx, conn, hello); err != nil {
		return err
	}
	var challenge trust.RelayFrame
	if err = wsjson.Read(ctx, conn, &challenge); err != nil {
		return err
	}
	certificate, err := x509.ParseCertificate(challenge.Certificate)
	if err != nil {
		return err
	}
	key, ok := certificate.PublicKey.(ed25519.PublicKey)
	if !ok || challenge.T != "AuthChallenge" || len(challenge.Nonce) != 32 || trust.Fingerprint(certificate.Raw) != enrollment.Fingerprint || !ed25519.Verify(key, oracle("gateway", hello, challenge, nil), challenge.Signature) {
		return fmt.Errorf("gateway identity rejected")
	}
	proof := trust.RelayFrame{T: "AuthProof", Signature: ed25519.Sign(authority.Certificate.PrivateKey.(ed25519.PrivateKey), oracle("replica", hello, challenge, nil))}
	if err = wsjson.Write(ctx, conn, proof); err != nil {
		return err
	}
	var accepted trust.RelayFrame
	if err = wsjson.Read(ctx, conn, &accepted); err != nil {
		return err
	}
	if accepted.T != "Authenticated" || !ed25519.Verify(key, oracle("accepted", hello, challenge, accepted.Principal), accepted.Signature) {
		return fmt.Errorf("capture not authenticated")
	}
	conn.CloseNow()
	fresh, err := publicDial(ctx, in, *enrollment)
	if err != nil {
		return err
	}
	defer fresh.CloseNow()
	if err = wsjson.Write(ctx, fresh, hello); err != nil {
		return err
	}
	var next trust.RelayFrame
	if err = wsjson.Read(ctx, fresh, &next); err != nil {
		return err
	}
	if string(next.Nonce) == string(challenge.Nonce) {
		return fmt.Errorf("nonce reused")
	}
	if err = wsjson.Write(ctx, fresh, proof); err != nil {
		return err
	}
	bounded, stop := context.WithTimeout(ctx, time.Second)
	defer stop()
	err = wsjson.Read(bounded, fresh, &accepted)
	if err == nil || errors.Is(err, context.DeadlineExceeded) {
		return fmt.Errorf("replay was not rejected")
	}
	return json.NewEncoder(os.Stdout).Encode(map[string]bool{"capture_authenticated": true, "fresh_nonce": true, "replay_rejected": true})
}
