package simapple

import (
	"bytes"
	"crypto/ecdh"
	"crypto/ed25519"
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"encoding/json"
	"net/http"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type IPhone struct {
	Signing    ed25519.PrivateKey
	Encryption *ecdh.PrivateKey
	Gateway    []byte
	ID         string
}

func NewIPhone() (*IPhone, error) {
	_, signing, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		return nil, err
	}
	encryption, err := ecdh.X25519().GenerateKey(rand.Reader)
	return &IPhone{Signing: signing, Encryption: encryption}, err
}

func (p *IPhone) Pair(client *http.Client, address, raw string) (int, error) {
	code, err := trust.DecodePairCode(raw)
	if err != nil {
		return 0, err
	}
	signing := p.Signing.Public().(ed25519.PublicKey)
	encryption := p.Encryption.PublicKey().Bytes()
	message := trust.IPhonePairMessage(code.ID, signing, encryption)
	key := sha256.Sum256(code.Secret)
	mac := hmac.New(sha256.New, key[:])
	mac.Write(message)
	rawRequest, err := json.Marshal(trust.IPhonePairRequest{ID: code.ID, SigningKey: signing, EncryptionKey: encryption, Proof: mac.Sum(nil), Signature: ed25519.Sign(p.Signing, message)})
	if err != nil {
		return 0, err
	}
	response, err := client.Post(address+"/v1/iphone/pair", "application/json", bytes.NewReader(rawRequest))
	if err != nil {
		return 0, err
	}
	defer response.Body.Close()
	var result struct {
		ID      string `json:"id"`
		Gateway []byte `json:"gateway_key"`
	}
	if response.StatusCode == 200 {
		err = json.NewDecoder(response.Body).Decode(&result)
		p.ID = result.ID
		p.Gateway = result.Gateway
	}
	return response.StatusCode, err
}

func (p *IPhone) Request(client *http.Client, address, path string, payload []byte) (*http.Response, error) {
	at := time.Now().Unix()
	body, err := json.Marshal(trust.IPhoneRequest{DeviceID: p.ID, At: at, Payload: payload, Signature: ed25519.Sign(p.Signing, trust.IPhoneMessage(path, at, payload))})
	if err != nil {
		return nil, err
	}
	return client.Post(address+path, "application/json", bytes.NewReader(body))
}
