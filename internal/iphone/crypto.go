package iphone

import (
	"crypto/aes"
	"crypto/cipher"
	"crypto/ecdh"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha256"
	"encoding/json"
	"io"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"golang.org/x/crypto/hkdf"
)

type Packet struct {
	LiveActivity json.RawMessage `json:"live_activity,omitempty"`
	EphemeralKey []byte          `json:"ephemeral_key"`
	Box          []byte          `json:"box"`
	Signature    []byte          `json:"signature"`
}

func PacketMessage(p Packet) []byte {
	return append(append([]byte("apple-bridge-iphone-command/1\x00"), p.EphemeralKey...), p.Box...)
}

func sharedCipher(secret []byte, id string) (cipher.AEAD, error) {
	key := make([]byte, 32)
	if _, err := io.ReadFull(hkdf.New(sha256.New, secret, []byte("apple-bridge-iphone/1"), []byte(id)), key); err != nil {
		return nil, err
	}
	block, err := aes.NewCipher(key)
	if err != nil {
		return nil, err
	}
	return cipher.NewGCM(block)
}

func Seal(public []byte, id string, plain []byte, sign func([]byte) ([]byte, error)) (Packet, error) {
	var packet Packet
	recipient, err := ecdh.X25519().NewPublicKey(public)
	if err != nil {
		return packet, err
	}
	key, err := ecdh.X25519().GenerateKey(rand.Reader)
	if err != nil {
		return packet, err
	}
	secret, err := key.ECDH(recipient)
	if err != nil {
		return packet, err
	}
	aead, err := sharedCipher(secret, id)
	if err != nil {
		return packet, err
	}
	nonce := make([]byte, aead.NonceSize())
	if _, err = rand.Read(nonce); err != nil {
		return packet, err
	}
	packet.EphemeralKey = key.PublicKey().Bytes()
	packet.Box = aead.Seal(nonce, nonce, plain, nil)
	packet.Signature, err = sign(PacketMessage(packet))
	return packet, err
}

func Open(private *ecdh.PrivateKey, id string, packet Packet, gateway ed25519.PublicKey) ([]byte, error) {
	if len(gateway) != 32 || !ed25519.Verify(gateway, PacketMessage(packet), packet.Signature) {
		return nil, contracts.ErrUnauthorized
	}
	public, err := ecdh.X25519().NewPublicKey(packet.EphemeralKey)
	if err != nil {
		return nil, err
	}
	secret, err := private.ECDH(public)
	if err != nil {
		return nil, err
	}
	aead, err := sharedCipher(secret, id)
	if err != nil {
		return nil, err
	}
	if len(packet.Box) < aead.NonceSize() {
		return nil, contracts.ErrInvalid
	}
	return aead.Open(nil, packet.Box[:aead.NonceSize()], packet.Box[aead.NonceSize():], nil)
}
