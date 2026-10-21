package trust

import (
	"context"
	"encoding/base32"
	"encoding/json"
	"net/url"
	"strconv"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/coder/websocket"
)

func relayURL(address string) (*url.URL, error) {
	u, err := url.Parse(address)
	if err != nil || u.Scheme != "https" || u.Host == "" || u.Hostname() == "" || u.User != nil || u.RawQuery != "" || u.Fragment != "" || u.Opaque != "" || u.Path != "" && u.Path != "/" {
		return nil, contracts.ErrInvalid
	}
	if u.Port() != "" {
		port, err := strconv.Atoi(u.Port())
		if err != nil || port < 1 || port > 65535 {
			return nil, contracts.ErrInvalid
		}
	}
	u.Path, u.RawPath = "", ""
	return u, nil
}
func encodePairCode(code PairCode) (string, error) {
	raw, err := json.Marshal(code)
	if err != nil {
		return "", err
	}
	return "ab1-" + base32.StdEncoding.WithPadding(base32.NoPadding).EncodeToString(raw), nil
}
func (a *Authority) PairRemote(address string, grants []contracts.Grant, ttl time.Duration) (string, error) {
	u, err := relayURL(address)
	if err != nil {
		return "", err
	}
	encoded, err := a.PairTTL(u.String(), grants, ttl)
	if err != nil {
		return "", err
	}
	code, err := DecodePairCode(encoded)
	if err != nil {
		return "", err
	}
	code.Remote = true
	return encodePairCode(code)
}
func (a *Authority) JoinRelay(ctx context.Context, conn *websocket.Conn, code PairCode) error {
	if !code.Remote || code.Dial != "" || code.ID == "" || len(code.Secret) != 20 || len(code.Fingerprint) != 64 {
		return contracts.ErrInvalid
	}
	if _, err := relayURL(code.Address); err != nil {
		return err
	}
	if time.Now().After(code.ExpiresAt) {
		return contracts.ErrExpired
	}
	principal, err := a.relayClient(ctx, conn, code.Fingerprint, &code)
	if err != nil {
		return err
	}
	return a.SaveEnrollment(Enrollment{Remote: true, PairingID: code.ID, ReplicaID: principal.ID, Address: code.Address, Fingerprint: code.Fingerprint, Grants: principal.Grants, Ceiling: principal.Grants})
}
