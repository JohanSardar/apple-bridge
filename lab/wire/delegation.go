package wire

import (
	"crypto/ed25519"
	"encoding/base64"
	"encoding/json"
	"fmt"
)

type Delegation struct {
	LocalLevel     string   `json:"local_level,omitempty"`
	Challenge      string   `json:"challenge,omitempty"`
	ReplicaID      string   `json:"replica_id"`
	LocalPrincipal string   `json:"local_principal"`
	LocalGrants    []string `json:"local_grants"`
	CommandID      string   `json:"command_id"`
	RequestHash    string   `json:"request_hash"`
	IAT            int64    `json:"iat"`
}

func SignDelegation(key ed25519.PrivateKey, value Delegation) (string, error) {
	if len(key) != ed25519.PrivateKeySize {
		return "", fmt.Errorf("invalid delegation fixture key")
	}
	raw, err := json.Marshal(value)
	if err != nil {
		return "", err
	}
	signature := ed25519.Sign(key, raw)
	return base64.RawURLEncoding.EncodeToString(raw) + "." + base64.RawURLEncoding.EncodeToString(signature), nil
}
