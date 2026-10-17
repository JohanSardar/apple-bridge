package world

import (
	"encoding/base32"
	"encoding/json"
	"fmt"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func (w *World) PairingChecks() error {
	remote := w.Level == "replica"
	binary := w.bridge
	if remote {
		binary = w.remoteBinary
	}
	home := filepath.Join(w.Dir, "pairing")
	if remote {
		if _, err := w.execute(true, "mkdir", "-p", home); err != nil {
			return err
		}
	}
	defer func() {
		if !remote {
			w.CleanupCredential(home)
		}
	}()
	code, err := w.execute(false, w.bridge, "pair", "--home", w.Gateway.Home, "--peer-addr", w.pairAddress, "--scope", "reminders:read,system:read")
	if err != nil {
		return err
	}
	decoded, err := trust.DecodePairCode(code)
	if err != nil {
		return err
	}
	decoded.Secret[0] ^= 1
	bad, _ := json.Marshal(decoded)
	wrong := "ab1-" + base32.StdEncoding.WithPadding(base32.NoPadding).EncodeToString(bad)
	if _, err = w.execute(remote, binary, "join", wrong, "--home", home); err == nil {
		return errUnexpectedPairing
	}
	if _, err = w.execute(remote, binary, "join", code, "--home", home); err != nil {
		return err
	}
	if _, err = w.execute(remote, binary, "join", code, "--home", home); err == nil {
		return errUnexpectedPairing
	}
	expired, err := w.execute(false, w.bridge, "pair", "--home", w.Gateway.Home, "--peer-addr", w.pairAddress, "--scope", "reminders:read", "--expires-in", "1ms")
	if err != nil {
		return err
	}
	time.Sleep(10 * time.Millisecond)
	if _, err = w.execute(remote, binary, "join", expired, "--home", home); err == nil {
		return errUnexpectedPairing
	}
	raw, err := base32.StdEncoding.WithPadding(base32.NoPadding).DecodeString(strings.TrimPrefix(expired, "ab1-"))
	if err != nil {
		return err
	}
	var expiredCode trust.PairCode
	if err = json.Unmarshal(raw, &expiredCode); err != nil {
		return err
	}
	expiredCode.ExpiresAt = time.Now().Add(time.Minute)
	raw, _ = json.Marshal(expiredCode)
	extended := "ab1-" + base32.StdEncoding.WithPadding(base32.NoPadding).EncodeToString(raw)
	if _, err = w.execute(remote, binary, "join", extended, "--home", home); err == nil {
		return errUnexpectedPairing
	}

	return nil
}

var errUnexpectedPairing = fmt.Errorf("invalid pairing accepted")

func (w *World) ReplicaIdentity(index int) (string, error) {
	home := w.Replicas[index].Home
	data, err := w.execute(w.Replicas[index].Remote, "cat", filepath.Join(home, "credentials.json"))
	if err != nil {
		return "", err
	}
	var document struct {
		Enrollment trust.Enrollment `json:"enrollment"`
	}
	err = json.Unmarshal([]byte(data), &document)
	return document.Enrollment.ReplicaID, err
}
func (w *World) RevokeReplica(index int) error {
	id, err := w.ReplicaIdentity(index)
	if err != nil {
		return err
	}
	_, err = w.execute(false, w.bridge, "revoke", id, "--home", w.Gateway.Home)
	return err
}
func (w *World) ScopeReplica(index int, scope string) error {
	id, err := w.ReplicaIdentity(index)
	if err != nil {
		return err
	}
	_, err = w.execute(false, w.bridge, "scope", id, "--scope", scope, "--home", w.Gateway.Home)
	return err
}

func (w *World) Principal(endpoint Endpoint) (trust.Principal, error) {
	data, err := w.execute(endpoint.Remote, "cat", filepath.Join(endpoint.Home, "credentials.json"))
	if err != nil {
		return trust.Principal{}, err
	}
	var document struct {
		Principals []trust.Principal `json:"principals"`
		Enrollment *trust.Enrollment `json:"enrollment"`
	}
	if err = json.Unmarshal([]byte(data), &document); err != nil {
		return trust.Principal{}, err
	}
	hash := trust.Fingerprint([]byte(endpoint.Token))
	for _, principal := range document.Principals {
		if principal.TokenHash == hash {
			if document.Enrollment != nil {
				principal.Grants = trust.EffectiveGrants(principal.Grants, document.Enrollment.Grants)
			}
			return principal, nil
		}
	}
	return trust.Principal{}, fmt.Errorf("world principal unavailable")
}
