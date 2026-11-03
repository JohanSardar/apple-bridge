package main

import (
	"crypto/ecdh"
	"crypto/ed25519"
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"encoding/json"
	"flag"
	"os"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func iphoneDevice(args []string) error {
	set := flag.NewFlagSet("iphone-device", flag.ContinueOnError)
	home := set.String("home", "", "isolated gateway home")
	output := set.String("output", "", "private device key file")
	if err := set.Parse(args); err != nil {
		return err
	}
	if *home == "" || *output == "" || set.NArg() != 0 {
		return contracts.ErrInvalid
	}
	authority, err := trust.Open(*home)
	if err != nil {
		return err
	}
	raw, err := authority.PairIPhoneCode("https://iphone.invalid", false, time.Minute)
	if err != nil {
		return err
	}
	code, err := trust.DecodePairCode(raw)
	if err != nil {
		return err
	}
	public, private, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		return err
	}
	encryption, err := ecdh.X25519().GenerateKey(rand.Reader)
	if err != nil {
		return err
	}
	message := trust.IPhonePairMessage(code.ID, public, encryption.PublicKey().Bytes())
	key := sha256.Sum256(code.Secret)
	mac := hmac.New(sha256.New, key[:])
	mac.Write(message)
	phone, err := authority.PairIPhone(trust.IPhonePairRequest{ID: code.ID, SigningKey: public, EncryptionKey: encryption.PublicKey().Bytes(), Proof: mac.Sum(nil), Signature: ed25519.Sign(private, message)})
	if err != nil {
		return err
	}
	record, err := json.Marshal(struct {
		DeviceID    string `json:"device_id"`
		SigningSeed []byte `json:"signing_seed"`
	}{phone.ID, private.Seed()})
	if err != nil {
		return err
	}
	return os.WriteFile(*output, record, 0o600)
}
