package simapple

import (
	"bytes"
	"crypto/ecdh"
	"crypto/ed25519"
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base32"
	"encoding/json"
	"fmt"
	"net/http"
	"strconv"
	"strings"
	"time"
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
	var code struct {
		ID     string `json:"id"`
		Secret []byte `json:"secret"`
	}
	if !strings.HasPrefix(raw, "ab1-") {
		return 0, fmt.Errorf("invalid pairing code")
	}
	decoded, err := base32.StdEncoding.WithPadding(base32.NoPadding).DecodeString(strings.TrimPrefix(raw, "ab1-"))
	if err == nil {
		err = json.Unmarshal(decoded, &code)
	}
	if err != nil {
		return 0, err
	}
	signing := p.Signing.Public().(ed25519.PublicKey)
	encryption := p.Encryption.PublicKey().Bytes()
	message := append(append([]byte("apple-bridge-iphone-pair/1\x00"+code.ID+"\x00"), signing...), encryption...)
	key := sha256.Sum256(code.Secret)
	mac := hmac.New(sha256.New, key[:])
	mac.Write(message)
	rawRequest, err := json.Marshal(pairRequest{ID: code.ID, SigningKey: signing, EncryptionKey: encryption, Proof: mac.Sum(nil), Signature: ed25519.Sign(p.Signing, message)})
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
	body, err := json.Marshal(signedRequest{DeviceID: p.ID, At: at, Payload: payload, Signature: ed25519.Sign(p.Signing, append([]byte("apple-bridge-iphone/1\x00"+path+"\x00"+strconv.FormatInt(at, 10)+"\x00"), payload...))})
	if err != nil {
		return nil, err
	}
	return client.Post(address+path, "application/json", bytes.NewReader(body))
}

type pairRequest struct {
	ID            string `json:"id"`
	SigningKey    []byte `json:"signing_key"`
	EncryptionKey []byte `json:"encryption_key"`
	Proof         []byte `json:"proof"`
	Signature     []byte `json:"signature"`
}
type signedRequest struct {
	DeviceID  string `json:"device_id"`
	At        int64  `json:"at"`
	Payload   []byte `json:"payload"`
	Signature []byte `json:"signature"`
}
