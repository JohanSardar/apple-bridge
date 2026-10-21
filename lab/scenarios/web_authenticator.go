package scenarios

import (
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/binary"
	"encoding/json"
	"fmt"
)

type webAuthenticator struct {
	UserVerified bool
	Key          *ecdsa.PrivateKey
	ID           []byte
	Counter      uint32
}

func newWebAuthenticator() (*webAuthenticator, error) {
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return nil, err
	}
	id := make([]byte, 32)
	_, err = rand.Read(id)
	return &webAuthenticator{Key: key, ID: id, UserVerified: true}, err
}
func cborHead(major byte, size int) []byte {
	if size < 24 {
		return []byte{major<<5 | byte(size)}
	}
	if size < 256 {
		return []byte{major<<5 | 24, byte(size)}
	}
	return []byte{major<<5 | 25, byte(size >> 8), byte(size)}
}
func webCBOR(value any) []byte {
	switch v := value.(type) {
	case string:
		return append(cborHead(3, len(v)), []byte(v)...)
	case []byte:
		return append(cborHead(2, len(v)), v...)
	case int:
		if v >= 0 {
			return cborHead(0, v)
		}
		return cborHead(1, -1-v)
	case map[string]any:
		b := cborHead(5, len(v))
		for key, item := range v {
			b = append(b, webCBOR(key)...)
			b = append(b, webCBOR(item)...)
		}
		return b
	case map[int]any:
		b := cborHead(5, len(v))
		for key, item := range v {
			b = append(b, webCBOR(key)...)
			b = append(b, webCBOR(item)...)
		}
		return b
	}
	panic(fmt.Sprintf("unsupported authenticator value %T", value))
}
func (a *webAuthenticator) credential(create bool, challenge, origin, rp string) (map[string]any, error) {
	kind := "webauthn.get"
	if create {
		kind = "webauthn.create"
	}
	client, _ := json.Marshal(map[string]any{"type": kind, "challenge": challenge, "origin": origin, "crossOrigin": false})
	rpHash := sha256.Sum256([]byte(rp))
	auth := append([]byte{}, rpHash[:]...)
	flags := byte(5)
	if !a.UserVerified {
		flags = 1
	}
	if create {
		flags |= 64
	}
	a.Counter++
	auth = append(auth, flags)
	auth = binary.BigEndian.AppendUint32(auth, a.Counter)
	encode := base64.RawURLEncoding.EncodeToString
	response := map[string]any{"clientDataJSON": encode(client)}
	if create {
		auth = append(auth, make([]byte, 16)...)
		auth = binary.BigEndian.AppendUint16(auth, uint16(len(a.ID)))
		auth = append(auth, a.ID...)
		cose := map[int]any{1: 2, 3: -7, -1: 1, -2: a.Key.X.FillBytes(make([]byte, 32)), -3: a.Key.Y.FillBytes(make([]byte, 32))}
		auth = append(auth, webCBOR(cose)...)
		response["attestationObject"] = encode(webCBOR(map[string]any{"fmt": "none", "attStmt": map[string]any{}, "authData": auth}))
		response["transports"] = []string{"internal"}
	} else {
		clientHash := sha256.Sum256(client)
		signed := sha256.Sum256(append(append([]byte{}, auth...), clientHash[:]...))
		signature, err := ecdsa.SignASN1(rand.Reader, a.Key, signed[:])
		if err != nil {
			return nil, err
		}
		response["authenticatorData"] = encode(auth)
		response["signature"] = encode(signature)
		response["userHandle"] = encode([]byte("apple-bridge-owner"))
	}
	return map[string]any{"id": encode(a.ID), "rawId": encode(a.ID), "type": "public-key", "response": response, "clientExtensionResults": map[string]any{}}, nil
}
