package simapple

import (
	"crypto/aes"
	"crypto/cipher"
	"crypto/ecdh"
	"crypto/ed25519"
	"crypto/hmac"
	"crypto/sha256"
	"encoding/json"
	"fmt"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type IPhonePacket struct {
	EphemeralKey []byte          `json:"ephemeral_key"`
	Box          []byte          `json:"box"`
	Signature    []byte          `json:"signature"`
	LiveActivity json.RawMessage `json:"live_activity,omitempty"`
}

type IPhoneJob struct {
	Command contracts.Command `json:"command"`
	Hash    string            `json:"request_hash"`
	Expires int64             `json:"expires"`
}

type IPhoneResult struct {
	ID    string                 `json:"command_id"`
	Hash  string                 `json:"request_hash"`
	State contracts.CommandState `json:"state"`
	Data  json.RawMessage        `json:"data"`
}

func (p *IPhone) Open(packet IPhonePacket) ([]byte, error) {
	message := append(append([]byte("apple-bridge-iphone-command/1\x00"), packet.EphemeralKey...), packet.Box...)
	if len(p.Gateway) != ed25519.PublicKeySize || len(packet.Box) < 28 || !ed25519.Verify(p.Gateway, message, packet.Signature) {
		return nil, fmt.Errorf("invalid phone packet")
	}
	public, err := ecdh.X25519().NewPublicKey(packet.EphemeralKey)
	if err != nil {
		return nil, err
	}
	shared, err := p.Encryption.ECDH(public)
	if err != nil {
		return nil, err
	}
	extract := hmac.New(sha256.New, []byte("apple-bridge-iphone/1"))
	extract.Write(shared)
	expand := hmac.New(sha256.New, extract.Sum(nil))
	expand.Write(append([]byte(p.ID), 1))
	block, err := aes.NewCipher(expand.Sum(nil))
	if err != nil {
		return nil, err
	}
	aead, err := cipher.NewGCM(block)
	if err != nil {
		return nil, err
	}
	return aead.Open(nil, packet.Box[:12], packet.Box[12:], nil)
}
