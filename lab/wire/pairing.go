package wire

import (
	"encoding/base32"
	"encoding/json"
	"errors"
	"net/url"
	"strconv"
	"strings"
	"time"
)

type PairCode struct {
	Purpose     string    `json:"purpose,omitempty"`
	Remote      bool      `json:"remote,omitempty"`
	Dial        string    `json:"dial,omitempty"`
	ID          string    `json:"id"`
	Address     string    `json:"address"`
	Fingerprint string    `json:"fingerprint"`
	Secret      []byte    `json:"secret"`
	ExpiresAt   time.Time `json:"expires_at"`
}

func DecodePairCode(encoded string, now time.Time) (PairCode, error) {
	var code PairCode
	invalid := errors.New("invalid pair code")
	if !strings.HasPrefix(encoded, "ab1-") {
		return code, invalid
	}
	raw, err := base32.StdEncoding.WithPadding(base32.NoPadding).DecodeString(strings.TrimPrefix(encoded, "ab1-"))
	if err != nil || json.Unmarshal(raw, &code) != nil || code.ID == "" || code.Address == "" || len(code.Fingerprint) != 64 || len(code.Secret) != 20 {
		return code, invalid
	}
	if code.Remote {
		address, err := url.Parse(code.Address)
		if err != nil || address.Scheme != "https" || address.Host == "" || address.Hostname() == "" || address.User != nil || address.RawQuery != "" || address.Fragment != "" || address.Opaque != "" || address.Path != "" && address.Path != "/" || code.Dial != "" {
			return code, invalid
		}
		if address.Port() != "" {
			port, err := strconv.Atoi(address.Port())
			if err != nil || port < 1 || port > 65535 {
				return code, invalid
			}
		}
	}
	if now.After(code.ExpiresAt) {
		return code, errors.New("expired pair code")
	}
	return code, nil
}
