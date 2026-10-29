package trust

import (
	"crypto/ed25519"
	"crypto/hmac"
	"crypto/sha256"
	"encoding/json"
	"net/http"
	"strconv"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type IPhonePairRequest struct {
	ID            string `json:"id"`
	SigningKey    []byte `json:"signing_key"`
	EncryptionKey []byte `json:"encryption_key"`
	Proof         []byte `json:"proof"`
	Signature     []byte `json:"signature"`
}

func IPhonePairMessage(id string, signing, encryption []byte) []byte {
	b := append([]byte("apple-bridge-iphone-pair/1\x00"+id+"\x00"), signing...)
	return append(b, encryption...)
}

func (a *Authority) PairIPhone(req IPhonePairRequest) (contracts.IPhoneDevice, error) {
	var result contracts.IPhoneDevice
	if len(req.SigningKey) != ed25519.PublicKeySize || len(req.EncryptionKey) != 32 {
		return result, contracts.ErrInvalid
	}
	message := IPhonePairMessage(req.ID, req.SigningKey, req.EncryptionKey)
	if !ed25519.Verify(req.SigningKey, message, req.Signature) {
		return result, contracts.ErrUnauthorized
	}
	err := a.update(func(d *credentials) error {
		pair, ok := d.Pairings[req.ID]
		if !ok || pair.Purpose != "iphone" || pair.Used || pair.Attempts >= 5 || time.Now().After(pair.ExpiresAt) {
			return contracts.ErrUnauthorized
		}
		mac := hmac.New(sha256.New, pair.Key)
		mac.Write(message)
		if !hmac.Equal(mac.Sum(nil), req.Proof) {
			return contracts.ErrUnauthorized
		}
		result.ID = "iphone:" + contracts.NewID("")
		d.Principals = append(d.Principals, Principal{ID: result.ID, Kind: "iphone", Grants: []contracts.Grant{}, Revision: 1, IPhoneSigningKey: req.SigningKey, IPhoneEncryptionKey: req.EncryptionKey, CreatedAt: time.Now().UTC()})
		pair.Used = true
		pair.Key = nil
		d.Pairings[req.ID] = pair
		return nil
	})
	return result, err
}

func (a *Authority) IPhones() ([]contracts.IPhoneDevice, error) {
	d, err := a.load()
	result := []contracts.IPhoneDevice{}
	for _, p := range d.Principals {
		if p.Kind == "iphone" {
			result = append(result, contracts.IPhoneDevice{ID: p.ID})
		}
	}
	return result, err
}

type IPhoneRequest struct {
	DeviceID  string `json:"device_id"`
	At        int64  `json:"at"`
	Payload   []byte `json:"payload"`
	Signature []byte `json:"signature"`
}

func IPhoneMessage(path string, at int64, payload []byte) []byte {
	return append([]byte("apple-bridge-iphone/1\x00"+path+"\x00"+strconv.FormatInt(at, 10)+"\x00"), payload...)
}

func (a *Authority) VerifyIPhone(path string, req IPhoneRequest) (Principal, error) {
	p, err := a.Principal(req.DeviceID)
	if err != nil || p.Kind != "iphone" || len(p.IPhoneSigningKey) != ed25519.PublicKeySize || time.Since(time.Unix(req.At, 0)) > time.Minute || time.Until(time.Unix(req.At, 0)) > time.Minute || !ed25519.Verify(p.IPhoneSigningKey, IPhoneMessage(path, req.At, req.Payload), req.Signature) {
		return Principal{}, contracts.ErrUnauthorized
	}
	return p, nil
}

func (a *Authority) IPhoneHandler() http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPost {
			http.Error(w, "method", 405)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		if r.URL.Path == "/v1/iphone/pair" {
			var req IPhonePairRequest
			if json.NewDecoder(http.MaxBytesReader(w, r.Body, 8192)).Decode(&req) != nil {
				http.Error(w, "invalid", 400)
				return
			}
			result, err := a.PairIPhone(req)
			if err != nil {
				http.Error(w, "unauthorized", 401)
				return
			}
			json.NewEncoder(w).Encode(result)
			return
		}
		if r.URL.Path == "/v1/iphone/status" {
			var req IPhoneRequest
			if json.NewDecoder(http.MaxBytesReader(w, r.Body, 8192)).Decode(&req) != nil {
				http.Error(w, "invalid", 400)
				return
			}
			p, err := a.VerifyIPhone(r.URL.Path, req)
			if err != nil {
				http.Error(w, "unauthorized", 401)
				return
			}
			json.NewEncoder(w).Encode(contracts.IPhoneDevice{ID: p.ID})
			return
		}
		http.NotFound(w, r)
	})
}

func (a *Authority) PairIPhoneCode(address string, remote bool, ttl time.Duration) (string, error) {
	u, err := relayURL(address)
	if err != nil {
		return "", err
	}
	raw, err := a.pairPurpose(u.String(), "", nil, ttl, "iphone")
	if err != nil {
		return "", err
	}
	code, err := DecodePairCode(raw)
	if err != nil {
		return "", err
	}
	code.Remote = remote
	return encodePairCode(code)
}
